---
title: "0021 — Real-time chart push over the MCP SSE stream"
description: "Push recomputed series tails for Binance charts over GET /mcp as notifications/chart_tick, with tail polling as the fallback."
sidebar:
  order: 21
---

**Status:** Accepted
**Date:** 2026-11-05
**Amended by:** [ADR-0043](/chartgen/decisions/0043-multi-chart-grid/) (the cap per client address goes from 8 to 32) and [ADR-0044](/chartgen/decisions/0044-sign-in-through-github-and-google/) (with sign-in on, the per-client cap counts per user)

Supersedes the "push is deferred" part of
[ADR-0009](/chartgen/decisions/0009-live-tail-polling-and-history-paging/).
Tail polling, `warmup` and history paging from ADR-0009 stay as they are.

## Context and Problem Statement

The web chart polls `generate_chart(format=series, bars=2, warmup=300)`
every 2 s for 24/7 symbols (ADR-0009). TradingView moves the forming
candle several times a second. A 2 s poll is visibly slower, and each poll
costs a full upstream fetch of `2 + warmup` bars plus an indicator
recompute, for every open chart.

ADR-0009 deferred server push because it needs per-subscriber state,
indicator recomputation per push, and reconnect handling. The server
already has most of the parts: a Binance WebSocket feed with reconnect,
backoff and REST backfill (`src/feed.rs`), and the MCP Streamable HTTP SSE
stream on `GET /mcp`.

Indicators are computed on the server (ADR-0003, ADR-0007). A raw kline
tick carries only OHLCV.

## Decision Drivers

- Sub-second updates for Binance symbols.
- Keep indicator math on the server, and keep the client's merge path:
  a pushed update must merge exactly like a polled tail.
- Multi-user server with OAuth: subscriptions are per token, bounded, and
  end when the client goes away.
- Must work under plain `--serve`, without the trading engine.
- E2E tests run offline (`CHARTGEN_DATA_SOURCE=sample`).

## Considered Options

### What to push

**Push OHLCV, the client polls for indicators.** Each tick tells the
client to poll the tail. Latency is still one round trip plus a fetch per
tick, and the poll load grows with the tick rate. Rejected.

**Push OHLCV, the client computes indicators.** Ruled out by ADR-0003 and
ADR-0007: the browser does no indicator math.

**Push recomputed series tails.** The server keeps a rolling window of
`2 + warmup` raw bars per chart, applies each feed event, recomputes the
tail exactly like `generate_chart(format=series, bars=2, warmup)` and pushes
it. The client feeds it into the existing `mergeTail` path unchanged.
Chosen.

### Where the ticks come from

**Binance kline stream only.** Binance sends kline updates every 2 s,
which is no faster than polling.

**Kline plus aggTrade on one combined stream.** Trades arrive many times a
second. They are folded into the forming candle (close, high, low,
volume), and each kline update replaces it with Binance's own values.
Chosen.

### Transport

**A new WebSocket endpoint.** A second transport with its own auth.
Rejected.

**The MCP SSE stream (`GET /mcp`).** Already authenticated with the bearer
token, already read by the MCP SDK the web app uses, and already used for
`notifications/alert_triggered`. Chosen.

## Decision Outcome

Chosen: **server-side recomputed tails over the MCP SSE stream, polling as
the fallback.**

- **Tools.** `subscribe_chart({ticker, timeframe, indicators, chart_style,
  warmup})` returns `{subscription_id}`. `unsubscribe_chart({subscription_id})`
  ends it. The notification is `notifications/chart_tick` with
  `{subscription_id, series}`, where `series` is a SeriesPayload tail.
- **Pushable.** Only Binance symbols (and, under the sample source, the same
  symbols with a synthetic feed every ~500 ms). Others get an error starting
  with `not pushable`, which the client takes as "poll".
- **Groups and feeds.** Subscriptions with equal arguments share a group:
  one rolling window, seeded through the normal fetch path, and one
  recompute per push. Groups on the same symbol and interval share one feed,
  so one Binance WebSocket serves all of them. A group's feed receiver is
  subscribed before seeding, so no event in between is lost. Seeding and
  every recompute run on the blocking thread pool, not on the async
  workers; one that fails or panics ends its group (its clients fall back
  to polling).
- **Throttle.** A group pushes at most once per 250 ms (500 ms when
  `warmup` is above 300, since the recompute grows with the window), always
  the newest state. The first push follows the seed immediately.
- **Scope.** A subscription belongs to the bearer token and the SSE
  stream, identified by token plus `Mcp-Session-Id`. The server mints a
  session id on `initialize`, and the SDK sends it on every request,
  including `GET /mcp`. The alert registry keys its SSE senders the same
  way, so two tabs of one token both get alerts.
- **Input.** The ticker must be 2 to 20 ASCII letters or digits before it
  reaches a feed and the Binance WebSocket URL. At most 10 indicators per
  subscription, `warmup` at most 1000.
- **Cleanup.** Unsubscribe or the SSE stream closing removes the
  subscription. The last subscription of a group stops the group. A feed
  without groups stays connected for 30 s and closes after that, so a
  quick unsubscribe and resubscribe does not reconnect the WebSocket. A
  reconnecting stream with the same key takes over its subscriptions.
- **Backpressure.** Each SSE stream has a bounded buffer of 256 frames.
  A stream that is full (its peer stopped reading) or closed is unlinked
  with all its subscriptions. Dead peers that never fill the buffer are
  found when the 30 s keepalive write fails.
- **Engine-independent.** The registry (`src/chart_push.rs`) lives in the
  HTTP server state, not in the trading engine, so plain `--serve` pushes.
- **Transport fix.** Every POST made only of notifications or responses
  (one, or a batch) is answered with `202 Accepted` and no body
  (previously `204` for `notifications/initialized`, and an error for
  others such as `notifications/cancelled`). The MCP SDK opens its
  `GET /mcp` stream only after a 202 for `notifications/initialized`.
- **Client.** After a full load the live feed subscribes. While pushes
  arrive it stops polling. No push for 6 s (3× the 2 s kline cadence), or a
  dropped stream, falls back to polling, and the subscription is retried
  with backoff (2 s, doubling to 60 s). A chart change resubscribes. A hidden
  tab unsubscribes and resubscribes when visible. A pushed tail that does
  not line up triggers one poll, which catches up or reloads (ADR-0009);
  any other poll that lands while subscribed is dropped, since it may be
  older than the pushes. A refusal that starts with `not pushable` or `push
  limit reached` makes the client poll that chart without retrying. The
  badge shows `live` for push and `live (polling)` for polling.

### Limits and deployment assumption

Push assumes the hosted deployment: one chartgen behind a TLS reverse
proxy (or tunnel) for a small number of users, where OAuth hands a token
to any client that completes the PKCE flow. Tokens are therefore free, and
the caps cannot rely on them alone:

| Limit | Value | Error |
|---|---|---|
| Subscriptions per token | 4 | `push limit reached: …` |
| Subscriptions per client address | 8 | `push limit reached: …` |
| Subscriptions per server | 64 | `push limit reached: …` |
| `subscribe_chart` calls per token | 10 per minute | `push limit reached: …` |
| Indicators per subscription | 10 | `push limit reached: …` |
| Pushes per group | 4/s (2/s above `warmup` 300) | — |
| Feed linger after its last group | 30 s | — |
| SSE buffer per stream | 256 frames | stream unlinked |

The client address is the TCP peer. chartgen trusts no forwarding header
(`X-Forwarded-For` and the like), because nothing guarantees a proxy in
front strips a client-supplied one. Behind a reverse proxy every client
therefore shares the proxy's address, and the per-address cap then works
as a second server-wide cap of 8. That suits a personal deployment. A
multi-tenant one needs a trusted proxy header first, in a later ADR.

## Consequences

**Positive.** Binance charts tick sub-second with correct indicators on
the forming bar. One WebSocket and one recompute serve every viewer of a
chart, instead of one upstream fetch per viewer per poll. The client merge
code is unchanged.

**Negative.**

- The server holds state per subscription group (a window of
  `2 + warmup` bars) and a task per group and feed.
- Trade folding is an approximation between kline updates: volume can be
  off until the next kline (at most 2 s), and a candle opened by a trade
  starts at that trade's price.
- A subscription is lost when its SSE stream drops. The client notices
  within 6 s and polls meanwhile.
- Yahoo symbols still poll.

**Neutral.** The cumulative-indicator seam of ADR-0009 also applies to
pushed tails, since each window warms up independently of the full load.

## More Information

- [ADR-0009 — Live tail polling and history paging](/chartgen/decisions/0009-live-tail-polling-and-history-paging/)
- [ADR-0002 — MCP transport: Streamable HTTP](/chartgen/decisions/0002-mcp-transport-streamable-http/)
- [`reference/mcp.md`](/chartgen/reference/mcp/#subscribe_chart--unsubscribe_chart)
- [Web guide: live updates](/chartgen/guides/web/#live-updates-and-history)
