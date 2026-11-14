---
title: "0073 — A symbol that is not trading has no market context"
description: "Binance still serves a delisted symbol's last bars. get_market_context answers {symbol, error} instead of a price when a Binance symbol's newest 4h bar opened more than 8 hours ago, or another symbol's newest bar is more than 7 days old; a wake-up about it goes without context."
sidebar:
  order: 73
---

**Status:** Accepted
**Date:** 2026-11-14
**Amends:** [ADR-0072](/chartgen/decisions/0072-wake-up-context-and-near-stop/) (what counts as a
symbol whose context can be read)

## Context and Problem Statement

ADR-0072 makes the context from a symbol's newest bars and answers `{symbol, error}` only when
they cannot be read. Binance, though, still serves the klines of a symbol it delisted from spot.
XMRUSDT was delisted in 2024, and its last 1d candle closed at 118.70 on 2024-02-20. In production,
`get_market_context` answered:

    XMRUSDT 118.7 | trend 4h down, 1d down | funding 0.1382% | F&G 13 extreme fear

That puts a 2024 price and 2024 trends next to a live funding rate and today's index, with nothing
to say so. The same holds for any delisted or halted symbol, and a wake-up's `context` would carry it
too. The 100eyes triage, which calls the tool for single coins, already drops coins that are not
trading on Binance spot, but the context must not depend on that.

## Decision Drivers

- A context describes the market now. A price from long ago must not read as a current one.
- No new data source and no extra request: decide from the bars the context reads anyway.
- No false alarm on a market that is shut for a weekend or a holiday.

## Considered Options

1. **The age of the newest bar, with a limit per kind of market** (chosen).
2. Binance's exchange information (the symbol's `status`, e.g. `TRADING`). An extra request, and
   there is nothing like it for Yahoo.
3. The 24h ticker's `closeTime` and trade count. Another request, and Binance only.

## Decision Outcome

A symbol is **not trading** when the newest 4h bar of its context's read is older than:

- **8 hours since it opened**, for a Binance symbol, which trades around the clock. The forming 4h
  bar opened at most 4 hours ago; one bar more is allowed for a late feed. Older means nothing traded
  for at least one whole bar.
- **7 days**, for any other symbol (stocks, ETFs, indices, FX through Yahoo), whose market shuts for
  weekends and holidays. A weekend with a holiday after it stays within that.

Such a symbol answers `{"symbol": "XMRUSDT", "error": "no bar since 2024-02-20T00:00:00Z: not
trading"}` with no price, trend or futures parts. The error is held for a minute like any other
answer. **A wake-up about it goes without `context`**, as for any context that could not be read.

### Consequences

- Good: a delisted or halted symbol no longer shows an old price as the current one, in the tool or
  in a wake-up.
- Good: no request is added; the check reads the bars already fetched.
- Neutral: a symbol read from Yahoo that trades around the clock (BTC-USD) gets the lenient 7-day
  limit.
- Out of scope: `get_quotes` has the same blind spot (XMRUSDT answers `last` 118.7, change 0). Its
  ticker carries a `closeTime` that could tell, but a limit there changes what the web watchlist
  shows for illiquid pairs, so it is a decision of its own. `generate_chart` draws old candles with
  their dates on the axis, which says what they are.

## Validation

- A test symbol under the test source whose bars end on 2024-02-20 answers the error; with the check
  disabled the test fails.
- The two limits are tested on both sides: a Binance bar 8 and 9 hours old, a stock's 4 and 8 days.
- A live call on the branch answers XMRUSDT with `no bar since 2024-02-20T00:00:00Z: not trading`,
  and BTCUSDT with its context as before.
