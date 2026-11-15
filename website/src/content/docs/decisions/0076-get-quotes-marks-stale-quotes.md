---
title: "0076 — get_quotes marks a quote that is not trading as stale"
description: "A quote whose market has not updated it for longer than that market is ever shut keeps its price, but its change is null and it carries stale: true and last_bar_at. The limit is 2 hours for Binance and Yahoo crypto, 3 days for FX and 4 days for everything else."
sidebar:
  order: 76
---

**Status:** Accepted
**Date:** 2026-11-15
**Amends:** [ADR-0011](/chartgen/decisions/0011-watchlist-quotes/) (what a quote answers)
**Follows:** [ADR-0073](/chartgen/decisions/0073-no-context-for-a-symbol-not-trading/), which left
`get_quotes` out of scope

## Context and Problem Statement

Binance still answers the 24 h ticker of a symbol it delisted from spot. `get_quotes` answered
XMRUSDT with `last` 118.7 and `change` 0, as if it were a current reading of a flat market. Its
ticker's `closeTime`, the time of its last update, was 2026-11-13 03:09 UTC, days before.
ADR-0073 made `get_market_context` refuse such a symbol, and named `get_quotes` as the same blind
spot, left for a decision of its own because a limit there changes what the web watchlist shows.

The trading agent calls `get_quotes` before every entry and every near-stop check, and for its
watchlist. Asked what it needs, it asked for the price to stay, with a flag, a time and no change,
and for the limits below. The person approved that.

## Decision Drivers

- An old price must not read as a current one, and a change of 0 must not read as a flat market.
- The web watchlist keeps every symbol it lists, illiquid pairs included: a row must not vanish.
- No extra request: each upstream already says when it last updated the quote.
- No false alarm on a market that is shut for a weekend or a holiday.

## Considered Options

1. **An error with no price**, as `get_market_context` answers since ADR-0073. A delisted symbol
   would show dashes in the watchlist like an unknown one, and the agent loses the last price,
   which it still wants for a position it holds.
2. **Keep the price and mark the quote stale** (chosen): `last` stays, `change` and `change_pct`
   are `null`, and the quote adds `"stale": true` and `last_bar_at`.

## Decision Outcome

A quote is **stale** when its last update is older than its market's limit at the time the call
is answered:

| Market | Last update | Stale after |
|---|---|---|
| Binance | the 24 h ticker's `closeTime` | more than 2 hours |
| Yahoo crypto (`instrumentType` `CRYPTOCURRENCY`) | `meta.regularMarketTime` | more than 2 hours |
| Yahoo FX (`CURRENCY`) | `meta.regularMarketTime` | more than 3 days |
| Yahoo stocks, ETFs, indices and any other type | `meta.regularMarketTime` | more than 4 days |

- **2 hours** for markets that trade around the clock: a live pair updates every few seconds, so
  two hours without one means nothing is trading.
- **4 days** for US stocks, ETFs and indices: the longest regular gap is a Friday close (16:00 ET)
  to the Tuesday open (09:30 ET) after a Monday holiday, about 3 days 17.5 hours.
- **3 days** for FX: the weekend gap is about 2 days.
- There is no limit per timeframe, because `get_quotes` has none.

A stale quote answers, for example:

```json
{ "symbol": "XMRUSDT", "last": 118.7, "change": null, "change_pct": null,
  "prev_close": 118.7, "high": 118.7, "low": 118.7, "volume": 0.0,
  "source": "binance", "stale": true, "last_bar_at": "2026-11-13T03:09:00Z" }
```

A fresh quote is unchanged and has neither field. A quote without an update time (a replay quote,
or an upstream answer that leaves it out) is never stale; a `sample` quote is always fresh.
Errors are unchanged. Staleness is judged when the call is answered, not when the quote was
fetched, so the 3 s quote cache cannot keep a quote fresh. The web watchlist keeps a stale row,
dims its price, shows dashes for the change, and names the time of the last update on hover.

### Consequences

- Good: a delisted or halted symbol no longer reads as a current, flat price, to the agent or in
  the watchlist, and its row stays.
- Good: no request is added; the time comes in the answer each upstream already gives.
- Neutral: a market shut for longer than its limit, such as an exchange on a week-long holiday,
  reads as stale until it opens again. That is what the quote is then: not updated.
- Neutral: `get_market_context` still answers an error for the same case (ADR-0073), with its own
  limits read from bars. The two tools answer different needs and keep different shapes.

## Validation

- The Binance and Yahoo fixtures parse their update time (`closeTime` in ms,
  `regularMarketTime` in seconds), and each Yahoo `instrumentType` gets its limit.
- Each limit is tested on both sides: exactly at the limit is fresh, a millisecond over is stale;
  the Friday-to-Tuesday weekend is fresh.
- The answer's shape: a stale quote has `null` changes, `stale` and `last_bar_at`; a fresh one
  has the nine fields it always had.
