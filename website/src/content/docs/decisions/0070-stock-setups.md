---
title: "0070 — Setups on US stocks, scored on Yahoo's 1m bars"
description: "A setup can be made on a US-listed stock or ETF as well as on a Binance symbol. The server polls Yahoo's 1m bars of the regular session and moves the setup with the same scorer. Expiry, a skip's bracket and the time stop count session time. An open past a target fills at the open. Stocks have their own cost assumption, and a split or a dividend during a setup adjusts it."
sidebar:
  order: 70
---

**Status:** Proposed
**Date:** 2026-11-14
**Amends:** [ADR-0059](/chartgen/decisions/0059-setups-paper-scored/) (a setup's symbol can be a US
stock or ETF read from Yahoo; Binance setups are unchanged) and
[ADR-0068](/chartgen/decisions/0068-replay-over-real-history/) (a replay and its fixture builder
take stock symbols)

## Context and Problem Statement

Hermes analyses stocks through chartgen's Yahoo source: quotes, charts and indicators for NVDA or
SPY. It cannot open a paper setup on one. `create_setup` refuses any symbol that is not a Binance
symbol, because the scorer reads closed 1m bars from the Binance WebSocket feed, and Yahoo has no
feed.

The question is how to score a stock setup as honestly as a Binance one. A stock trades in
sessions, gaps overnight and over weekends, and splits and pays dividends. Its costs also differ
from Binance's 5 bps fee and 2 bps slippage per side.

## Decision Drivers

- **Binance setups stay exactly as they are.** They are in production and Hermes relies on them.
  Their stored rows, their answers and `tests/fixtures/setup_responses.json` do not change.
- **One scorer.** A stock setup moves through `Record::apply_bar`, as a Binance setup and a replay
  do, so their results compare.
- **No invented prices.** A minute without a trade is not a bar, a closed market is not a gap, and
  a read that failed is retried, not skipped.
- **Measured, not assumed.** Yahoo's limits decide the bar size.
- **Small.** No new data source, and no rule that Binance setups would have to follow too.

## Yahoo's limits, measured

Measured on 2026-11-14, a Saturday, from the PC's home IP. There were 21 requests to
`query1.finance.yahoo.com/v8/finance/chart`, a few seconds apart, with one burst of 8 at one per
second.

| What | Measured |
|---|---|
| 1m history | **30 days.** A window starting 31 days back is refused: `422 Unprocessable Entity`, "The requested range must be within the last 30 days." A 7-day window ending 22 days back returns 1951 rows. |
| 1m rows per request | An 8-day window returned 2261 rows (6 sessions). The 7-day page that `yahoo_window` already uses stays. |
| Rows per session | 390 for every closed session of NVDA (09:30 to 15:59 New York). The latest session also carries a 16:00 row: volume 0, open = high = low = close = the official close. It is a last-price row, not a bar. |
| Minutes without a trade | Yahoo sends the row with nulls. It does not leave it out. NVDA and SPY had 0 null minutes; BRT, a thinly traded REIT, had 352 of 391. |
| Pre/post-market | Not sent unless asked for (`includePrePost`). `meta.currentTradingPeriod` gives the pre, regular and post windows. |
| Delay | `meta.exchangeDataDelayedBy` is absent for NVDA, SPY and BRT, so the data is not marked as delayed. BTC-USD, the only Yahoo market open on a Saturday, had its newest price 18 s before the request. The forming minute's row had a null close. **The lag of a US equity's 1m rows during a session was not measured, because the market was closed.** |
| Splits | `events=div,split` returns splits and dividends with a 1m read too. NVDA's 10:1 split read as `{date: 1718026200, numerator: 10, denominator: 1}`, dated at the session open it took effect. ORLY's 1h bars from before its 15:1 split of 2025-06-10 read about 91, not about 1370: **Yahoo adjusts intraday history for a split after the fact.** |
| Dividends | O's ex-date of 2026-10-30 came with its 1m read: `{amount: 0.272, date: 1793367000}`, which is the session open. |
| Rate limit | No 429 was seen in any of the 21 requests, including the burst. The threshold was not found inside the request budget, so the design stays far below the rates measured. |
| Instrument data | `meta.instrumentType` (EQUITY, ETF, INDEX, …), `exchangeName` (NMS for Nasdaq, NYQ for NYSE, PCX for NYSE Arca, …), `exchangeTimezoneName` and `currency`. |

## Considered Options

1. **Poll Yahoo's 1m bars for each stock symbol with open setups, and feed them to the same
   scorer, with session rules for stocks only.**
2. **Score stocks on a coarser bar, such as 5m or 15m, which has 60 days of history.** This loses
   the order of prices inside each bar more often, so more bars reach two levels at once and count
   as the worse one. It would also need a second scorer step size.
3. **Score stocks on the Binance rules unchanged, as if the market traded 24/7.** Every night would
   read as a gap, an expiry on a weekend would be exposed to Monday's open, and a skip's 24 bars
   would end in the middle of a weekend.

## Decision Outcome

Chosen: **option 1.**

### Markets

A stock setup takes a **US-listed stock or ETF**, as Yahoo reports it:
- `instrumentType` is EQUITY or ETF;
- the exchange is Nasdaq (NMS, NGM, NCM), NYSE (NYQ), NYSE American (ASE), NYSE Arca (PCX) or
  Cboe BZX (BTS);
- the time zone is America/New_York and the currency is USD.

`create_setup` checks this with one Yahoo read when the symbol is not already watched. Anything else
is refused with a hint:
- **Indices** (`^GSPC`) cannot be traded; use the ETF, such as SPY, QQQ or DIA.
- **OTC stocks** are refused because the cost assumption does not hold for them.
- **Other exchanges** each need their own calendar and cost assumption, and Yahoo marks many of
  them as delayed by 15 to 20 minutes.
- **Yahoo's crypto, FX and futures** are refused; crypto setups belong on Binance.

A symbol is a stock symbol when `detect_source` sends it to Yahoo, so every symbol that was
accepted before is still read the same way.

**A stock whose symbol ends like a Binance quote.** `detect_source` sends a symbol to Binance
when it ends in a quote asset such as BNB, so ABNB (Airbnb, Nasdaq) is routed to Binance.
- When Binance does not list such a symbol, `create_setup` tries the US-stock check before it
  refuses. If that passes, the setup is a stock setup, and its stored market routes it from then
  on: polling, restore and the server's watched symbols go by the record, not by the symbol.
- Its levels were checked with Binance's least risk, which is the stricter one.
- Charts still route ABNB to Binance, so a stock setup on it can take neither drawings nor
  `decision: skip`, which read the symbol's bars. That is a known limit of the chart routing,
  left as it is.
- A Binance symbol that Binance lists takes exactly the path it took before.

A stock setup's record carries `market: "us_equity"`. A Binance setup has no `market` field, so its
row and its answers are byte for byte as before.

### Bars: Yahoo's 1m bars of the regular session, polled

**The bar size is 1m.**
- 30 days of history covers a restart, a long outage and a setup's catch-up.
- A liquid stock has every minute of its session (390).
- At 1m the scorer and its rule for a bar that reaches two levels are the same as for Binance.
- 5m would have 60 days of history, but each bar would hide five times as much order.

**The source and the polling:**
- **Regular hours only.** Bars run from 09:30 to 16:00 New York, or to 13:00 on an early close.
  - Pre- and post-market trading is thin, and chartgen's stock charts do not show it.
  - A retail stop order works in regular hours by default.
  - A stop that would trade after hours therefore counts at the next open, which is the gap rule
    below.
  - The 16:00 last-price row is dropped.
- **Polling.** One task per stock symbol with open setups reads Yahoo every 60 s while a session
  is open, and once more after its close.
  - Each read starts at the oldest bar any of the symbol's setups still needs and runs to the newest
    minute that closed at least 60 s earlier. It asks for splits and dividends in the same request.
  - **What counts as read** is decided after the read, from what came back:
    - It runs to the end of the newest minute Yahoo sent a row for, null rows included.
    - If no session minute lies between that and the end asked for, it runs to the end.
    - It counts only once the setups it moved are stored. A failed write is a failed read, so the
      next poll reads the same minutes again.
    - A minute Yahoo has not published yet is therefore read by the next poll, not taken for a
      minute without a trade.
  - A minute applies only once it is 60 s old, so trades reported late have had a minute to arrive.
    The lag was not measured; see the delay row above.
  - One request covers 7 days. A setup further behind than that catches up one page per poll.
- **Load.** At most one request per symbol per minute, and none while the market is closed.
  Watched stock symbols count toward the 16 symbols the server watches for alerts and setups
  together (ADR-0042, ADR-0059), so the most is 16 requests a minute.
  - A failed read waits twice as long before the next one, up to 15 minutes.
  - A 429 waits at least 5 minutes.
- **No gap that is not one.** A failed read moves nothing and is read again, so no bar is skipped
  because of it. These are not gaps either:
  - a minute Yahoo sends as null, which had no trade;
  - nights, weekends and holidays.

  A setup's log gets a `gap` entry only for bars older than Yahoo's 1m history (29 days, a day
  short of the limit) that the server could no longer read. That is the same meaning a gap has for
  Binance.
- **An implausible open is held.** A read is held back, not applied and not counted as read, when
  both of these hold:
  - its first bar after the last close scored opens below 0.7 or above 1.4 times that close;
  - it holds no split.

  The next poll reads it again, and once Yahoo sends the split it is scored in the split's terms.
  The band takes in every split from 3:2 on (0.67) and every reverse split from 2:3 on (1.5). A
  5:4 split (0.8) is not caught. After 30 polls of the same first bar, half an hour at one a minute,
  it is taken as a real gap: scored as it is, at its open, and the server's log says so. A real 40 %
  gap is scored half an hour late. A split whose event Yahoo sends more than half an hour late is
  scored as a crash, and that log line is how an operator finds it.

### Sessions: weekends, holidays, expiry and counts in bars

The NYSE calendar already in `data.rs` decides sessions:
- its holidays and 13:00 early closes, read from nyse.com on 2026-11-06, through 2028;
- weekends.

Past 2028 it knows weekends only. So a stock setup whose expiry lies after 2028-12-31 is refused
until the table is extended. Closures that were not scheduled are not in the table, and their
missing bars are not gaps.

For a stock setup, these count session time, not 24/7 time:

- **Expiry.** A stock setup expires at **the last 1m close at or before `expires_at`**, after the
  stops and targets of that bar. A setup whose expiry falls on a Saturday therefore ends at Friday's
  close, not at Monday's open with the weekend's gap. An expiry on a holiday ends at the close
  before it, and an expiry at 12:00 on a thin stock whose last trade was at 11:40 ends at that
  trade.
  - The server applies the rule when the first bar after the expiry arrives, or when it has read
    past the expiry with no bar.
  - Unfilled, the setup scores 0 as before.
  - Binance setups keep their rule: the first bar that opens at or after the expiry, at its open.
    In a 24/7 market the two rules pick almost the same price, and Binance's rule is not changed.
- **A skip's expiry** is at the close of the 24th bar of its interval on the session grid, the
  bar forming now the first, and at most 90 days away. It is checked against the calendar's last
  year, as an agent's expiry is.
  - On a 1h chart, a session has 7 grid bars (09:30 to 15:30, the last a half hour), so 24 bars
    are about 3.4 sessions, not one day.
  - Bars of a day or longer count in sessions: a day is 1 session, a week 5 and a month 21.
- **The skip's ATR** comes from Yahoo's own bars of the interval, which are session bars already.
  Its least risk uses the stock costs below.
- **The time stop of shadow v3** (ADR-0066) closes at the end of the Nth bar of the interval after
  the fill bar.
  - Intraday bars follow the session grid, like Yahoo's own: they start at the open, and the
    session's last bar is shorter.
  - Day bars and longer count sessions, as above.
- **`bars_to_fill`, `bars_to_exit`, `bars_to_mae` and `bars_to_mfe`** count bars of the session
  grid, as the time stop does, to fractions of a bar. The short last bar of a session counts as a
  whole bar, and a night or a weekend is no bars.

### Gaps at the open

- **Stop.** An open beyond the stop exits at the open. That was already so for every setup.
- **Target.** **An open beyond a stock setup's target exits at the open**, which is the better
  price, as a resting limit order fills at a gap. The shadow variants follow the same rule for
  their targets.
  - Binance setups keep exiting at the target. A gap is rare in a 24/7 market, and their scores
    must not change.
- **Two levels in one bar.** When one bar reaches two levels, the worse one for the position still
  counts.
- **Entry.** Unchanged: a take fills at the zone's edge nearest the open, even when the open gapped
  through the zone in the position's favour. That never flatters a setup. A resting limit would
  have filled at the open, and the scorer leaves that out on purpose.

### Costs

A stock setup is scored with **1 bps fee and 3 bps slippage per side: 8 bps for the round trip**.
It is copied onto the setup, shown in its `costs` as for Binance, and named in the tool's
description.
- Most US brokers charge no commission. A per-share commission such as $0.005 a share is 0.25 to
  1 bps on a $50 to $200 stock, and the fees regulators charge on a sale are well under 1 bps.
- The spread of a liquid large cap or ETF is 1 to 2 bps. The 3 bps of slippage holds half the
  spread and some impact.
- The least risk is still 3 round trips, so 24 bps of the price.
- The assumption is for liquid names. A thinly traded stock is scored optimistically, and the server
  does not check liquidity.

### Splits and dividends

**Splits.** Yahoo adjusts history after a split. Once a split has taken effect, every bar it
returns, older ones included, is in the new terms. So when a read brings a split dated after a
setup was made, which that setup has not taken yet, the server:
1. divides every price of the setup by the ratio before it applies the read's bars: the levels,
   the pending levels, the fill, the exit, the last close, and the shadows' stops and exits;
2. writes an `adjusted` entry to the log, by the server.

Some cases need care:
- **Pending levels sent at or after the split's time are left as they are.** The agent read them
  from charts Yahoo had already adjusted.
- **Catch-up reads.** Yahoo answers in the terms of the latest split even for an old page. So a
  read of a page that ends more than an hour ago also asks Yahoo's daily chart for the splits from
  that page's end to now (one more request), and applies them before the page's bars.
- **An event that comes late** is held, as above.

R, MAE, MFE and the closest approach are ratios and do not move. The drawings stay as the agent
drew them. A setup made after the split read adjusted prices already, so it is left alone. A
replay's fixtures are recorded after the fact, so their bars are adjusted already and a replay
never sees a split.

**Dividends.** On the ex-date, a stock setup whose position is open is **credited the dividend
if long and charged it if short**, per share and in R, from its exit on. The amount is stored on the
setup with an `adjusted` entry in the log.
- A shadow part that exited before the ex-date does not get it.
- The levels are not moved. The ex-date's drop at the open is a price move like any other, and it
  can reach a long's stop: in the R that results, the dividend makes up for the drop.
- MAE and MFE stay price only.
- A replay fixture may hold the dividends of its window, and the builder records them.

### Contract

- **New record fields.** `market`, `last_close` (the last 1m close scored, for the expiry rule) and
  `adjustments` (splits and dividends applied).
  - Each is written only when it has a value, so a Binance setup is stored and answered exactly as
    before.
  - The log has one new kind, `adjusted`.
- **Tool descriptions.** The `create_setup` description and its `symbol` say that it takes a
  Binance symbol or a US stock or ETF. They name both cost assumptions, and the error hints name
  the supported markets.
- **Version.** It goes to 0.8.0.

### Replay

A replay's fixtures may hold stock symbols. For those:
- the session rules above apply;
- `Replay::bars` builds intraday bars on the session grid, as Yahoo does, so a skip's ATR in a
  replay is the one it would be live.

`chartgen replay-fixtures` takes stock symbols too:
- It reads Yahoo's 1m bars of the regular session in 7-day pages, with 2 s between requests.
- It records the window's dividends.
- It refuses a window older than Yahoo's 30 days.
- It refuses a symbol that `create_setup` would refuse, with the same check of Yahoo's metadata.
- A symbol whose quote suffix is not in `detect_source`'s list, such as BTCEUR or ETHTRY, is now
  read from Yahoo as a stock. Before, it was read from Binance. A Binance fixture of such a symbol
  is bucketed on the US session grid.
- It refuses `--shift-days` for a stock, since a shifted session would land on a holiday or a
  weekend that the calendar knows.

### Alerts tied to setups

Setup status changes are logged and wake the agent (ADR-0061) for stock setups, as they do for
Binance setups. Server alerts on stock symbols are not part of this decision: server alerts stay
on Binance symbols.

## Consequences

- Good: Hermes can test a stock thesis on paper with the scorer, costs rules and track record it
  has for crypto, and a replay can run it over the last 30 days.
- Good: a setup's result never comes from a minute that did not trade or a gap that was not one.
  A split is not read as a crash unless Yahoo sends its event more than half an hour after the
  bars; the server's log then says so.
- Bad: a status moves after the price by 1 to 2 minutes of polling and settling, plus Yahoo's own
  lag, which has not been measured yet. Wake-ups arrive that much later than for Binance.
- Bad: the lag of Yahoo's equity 1m data was not measured during a session. The settling margin
  of 60 s is a guess, to be checked on a trading day. A trade reported after a minute has been
  applied does not change that minute's bar here.
- Bad: costs assume a liquid name. Holidays past 2028 need the table extended. Server alerts are
  still Binance only.
- Bad: up to 16 Yahoo requests a minute from the server's IP, on top of what charts read. Yahoo's
  limit is undocumented, and none was met at these rates.

## Testing

- **Unit tests.**
  - The session calendar: a weekend, Thanksgiving 2026 and the early close after it, and session
    time across a weekend.
  - Expiry at the last close before a weekend, a holiday and a thin stretch.
  - The skip's session expiry, and the time stop across a night.
  - An open past a target, for the canonical setup and for each shadow, against an unchanged
    Binance case.
  - Stock costs and their least risk.
  - Splits scale every price once. Dividends are credited to a long and charged to a short, and
    left out for a shadow part that exited before the ex-date.
  - Parsing a recorded Yahoo 1m answer, with its null minutes, its 16:00 row and its events.
- **Service tests.** A failed read moves nothing and makes no gap. Null minutes and nights make no
  gap. A setup further back than Yahoo's reach gets a gap. A non-US symbol, an index and an OTC
  symbol are refused.
- **Replay fixtures (`tests/fixtures/replay_stocks/`).** NVDA's real 1m bars from 2026-11-06 to
  2026-11-13, recorded with the builder:
  - an expiry across the weekend that ends at Friday's close;
  - a long stopped by Monday's 7 % gap at the open, at that open;
  - a 1h skip whose bracket comes from session bars (09:30 to 15:30, the last a half hour) and
    whose expiry is the close of the 24th bar of the session grid.

  O's real 1m bars from 2026-10-29 to 2026-10-30 (`tests/fixtures/replay_stocks_dividend/`),
  with its dividend of 0.272, pay a long that was held over the ex-date.
- **The holiday.** No NYSE holiday lies inside Yahoo's 30 days of 1m history: the last was
  2026-09-07 and the next is 2026-11-26. So the holiday test builds its bars in code from the
  NVDA fixture's recorded sessions of 2026-11-10 to 2026-11-13. It moves them 14 days onto
  Thanksgiving week, removes the holiday's session and cuts Friday at its 13:00 early close. The
  prices are real; the dates are not. A long expiring on Thanksgiving ends at Wednesday's close,
  and one expiring on Friday afternoon ends at the 12:59 bar.
- **Binance.** `tests/fixtures/setup_responses.json` is unchanged and its test still passes.
