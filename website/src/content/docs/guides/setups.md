---
title: Setups
description: An agent stores a trading setup with a thesis and levels, and the server scores it on paper. A replay server runs the same tools over fixture bars, with the same answers every time.
---

A setup is a record an AI agent writes: what it expects, where it would get in, where it is
wrong, where it would take profit, and until when. The server then follows the price and scores
the setup **on paper**. No order is ever sent. The agent cannot set or change an outcome, so
its decisions get a track record a person can trust
([ADR-0059](/chartgen/decisions/0059-setups-paper-scored/)).

Setups are off until the operator sets [`CHARTGEN_SETUPS=1`](/chartgen/guides/deploy/#setups).
They run on Binance symbols and over the HTTP transport, where a call has an owner. A headless agent
needs [durable tokens](/chartgen/reference/oauth/#refresh-tokens) for them: a setup belongs to
the owner its token resolves to, which with sign-in off is the OAuth client, so an agent whose
token does not outlive a restart loses sight of its setups until someone authorizes it again. The tools
are in the [MCP reference](/chartgen/reference/mcp/#setups-chartgen_setups1). The web app does
not show setups yet.

## A setup

```json
{
  "symbol": "BTCUSDT",
  "interval": "1h",
  "direction": "long",
  "thesis": "Why, in plain text.",
  "entry": { "low": 60000, "high": 60200 },
  "invalidation": 59400,
  "targets": [61200, 62000],
  "expires_at": "2026-12-01T12:00:00Z",
  "drawings": [
    { "tool": "horizontalStraightLine", "points": [{ "time": "2026-11-30T08:00:00Z", "snap": "low" }] }
  ]
}
```

- **It must be able to be wrong.** A setup without an `invalidation` and an `expires_at` is
  refused. The invalidation is below the entry zone of a long and above that of a short, and at
  least 42 bps of the price away from it (three times the assumed cost of getting in and out).
- **All or nothing.** If anything is wrong, nothing is stored, and the error names each field
  with a hint.
- **Drawings** are the ones [`generate_chart`](/chartgen/reference/mcp/#drawings) takes, checked
  by the same rules on the bars of the setup's symbol and interval.

## How it is scored

The server moves the status from **closed 1m bars**: `watching`, then `triggered` when a bar
reaches the entry zone, then `won` at the first target, `invalidated` at the invalidation, or
`expired`. So a status can be up to a minute behind the price.

The result is in **R**: the move from the fill to the exit, in units of the distance from the
fill to the invalidation, less an assumed 5 bps fee and 2 bps slippage on each side. A setup
that was never filled scores 0.

A 1m bar does not say in which order it traded. **Where a bar is ambiguous, the worse outcome
for the agent counts:**

| The bar reaches | Counts as |
|---|---|
| The invalidation and the target | Invalidated |
| The entry zone and the invalidation, while watching | Filled and invalidated: a loss |
| The entry zone and the target, while watching | Filled only; that bar cannot win |
| The invalidation, opening beyond it or never reaching the zone | Invalidated, never filled, 0 R |

## Changing a setup

Nothing is edited in place. `revise_setup` and `cancel_setup` need a reason, and each adds an
entry to the setup's log with what it was and what it became. `get_setup` returns the whole log,
the server's own status moves included.

- The thesis and drawings change at once.
- New levels, and a cancel, **count from the next 1m bar**. The bar forming now still meets the
  setup as it was, so a fill or a stop inside it stands. Levels not yet in effect are under
  `pending`.
- Once a setup is filled its levels are fixed.
- Cancelling a filled setup closes it at the next bar's open and scores it.

## Skips

A decision **not** to trade is worth recording too. `decision: "skip"` stores it with a
direction and a thesis and no levels. The server scores it as if it had been taken, with the
same bracket for every skip: in at the next bar's open, out one ATR(14) against it or two with
it. On a quiet chart, where one ATR is less than the least risk a setup may have (42 bps), the
stop is that far and the target twice that; `bracket` on the setup says which applied (`atr` or
`floor`; `agent` for a setup with your own levels). A skip has an expiry after 24 bars of the setup's interval, at most 90 days. A skip cannot be
cancelled.

## Limits

Per owner: 20 open setups, 500 in total, open setups on 5 symbols, 50 revisions per setup, 10
changes a minute, and 4 MiB for setups and their logs. Closed setups are kept; at a limit a new
one is refused and nothing old is dropped. `list_setups` returns the limits.

## When the server was away

After a restart or a dropped feed the server reads the 1m bars a setup missed, up to 1000 of
them, and scores them in order. If it cannot, the log gets a `gap` entry that names the stretch
that was not scored, and the setup's `gap` field is true from then on. Treat a result with
`gap: true` as incomplete.

If the operator takes your account off the server's list, your setups are no longer scored. You
can still read them. One that runs out meanwhile is marked `expired` with `gap: true`: at 0 R if
it was never filled, and with no result (`result_r: null`) if it was, since no price was read.

## Replay

A replay server answers the same tools from **fixture bars**, as of a time each call names. It
is for evaluating an agent on a scenario that can be run again: the same calls give the same
answers, byte for byte, on every run.

```bash
CHARTGEN_REPLAY_FIXTURES=tests/fixtures/replay chartgen --serve --port 9315
```

A fixture is one JSON file per symbol, of 1m bars in order:

```json
{ "symbol": "BTCUSDT", "bars": [[1767225600, 60000.0, 60024.0, 59976.0, 60000.0, 10.0]] }
```

Each bar is `[open time in unix seconds, open, high, low, close, volume]`. The files in
`tests/fixtures/replay/` are synthetic.

- **Every call needs `as_of`**: unix seconds, unix ms or an ISO 8601 time. It is taken by the
  five setup tools and by `generate_chart`, `inspect_chart`, `get_indicators` and `get_quotes`.
- **Nothing after `as_of` is used.** A 1m bar counts only if it had closed by then, and every
  other interval is built from those bars. The bar still forming at `as_of` is not there.
- **Time only moves forward.** The server has one clock for all tools and all callers. A call
  with an `as_of` before the latest one the server has been sent is refused, whoever sent it.
  One process is one run: run two evaluations as two servers.
- **Everything it stores lives in memory**: setups, and the OAuth clients that register with
  it. A replay server opens no database file, not `chartgen.db` either, and all of it is gone
  when the process ends. To run a scenario again, restart the server. Ids count per caller: `setup-000001`, and so on.
- **What is refused:** `get_footprint`, `get_tpo`, `search_symbols`, chart subscriptions, the
  alert tools, and the indicators that fetch a series of their own (funding rate, open interest,
  long/short ratio, fear and greed).
- `get_quotes` returns the close of the newest closed 1m bar as `last`, and its 24-hour fields
  from the 1m bars of the 24 hours before `as_of`.
- A replay server is nothing else: it refuses to start with `--trade`, `--mcp`,
  `CHARTGEN_SERVER_ALERTS=1`, `CHARTGEN_SETUPS=1` or a sign-in provider. On any other server
  `as_of` is refused.
