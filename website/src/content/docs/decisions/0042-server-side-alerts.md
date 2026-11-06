---
title: "0042 — Alerts that run on the server, with webhooks"
description: "A price alert can run on the server with no browser open and notify a webhook. It reads the shared Binance feeds and follows the browser's fire rules. An operator allowlist decides who gets webhooks, which can only reach public HTTPS addresses."
sidebar:
  order: 42
---

**Status:** Accepted
**Date:** 2026-11-06
**Amends:** [ADR-0020](/chartgen/decisions/0020-client-side-price-alerts/) (an alert may now run without a tab), [ADR-0034](/chartgen/decisions/0034-drawing-and-indicator-alerts/) (its sources stay browser-only)

## Context and Problem Statement

Alerts run in an open browser tab ([ADR-0020](/chartgen/decisions/0020-client-side-price-alerts/)):
close the tab and nothing fires, and nothing can reach another system. People
want an alert to fire with no browser open and to call a webhook.

A server like this is public, may sit on a home network, and its registration is
open and auto-approved ([ADR-0036](/chartgen/decisions/0036-own-origin-invalid-client-redirect/)).
So a stranger can create alerts, and a webhook URL is a request the server makes
on a stranger's behalf. Where do prices come from without adding uncontrolled load
on Binance, how do the fire rules stay the same as the browser's, and what can a
webhook reach?

## Decision Drivers

- Must not become a way to reach the home network or a cloud metadata service.
- Must not add exchange load that grows with the number of alerts. The server's
  IP can be banned.
- A browser alert and a server alert with the same settings must fire alike.
- No fire twice across a restart, and no fire lost to a redeploy.
- Off until someone decides to turn it on.
- Client ids are free (ADR-0036), so a limit per client bounds a mistake, not an attacker.
  Who may make this server send requests has to be the operator's decision.

## Considered Options

1. **Reuse `alerts.json` and the engine behind `--trade`.** It keeps one flat list,
   with no owner, watches the one symbol and interval the process started with,
   reads closed bars only, and removes an alert when it fires. `--serve` does not
   run it. Making it multi-tenant would replace all of that and risk the trading
   path.
2. **Poll prices over REST.** Simple, and easy to put behind the Binance gate, but
   the load is continuous and grows with the symbols watched.
3. **A new store and evaluator on the chart hub's feeds (chosen).** One WebSocket
   per symbol and interval, already shared by every chart of it.

## Decision Outcome

Chosen: option 3, in `src/server_alerts.rs` and `src/webhook.rs`, on only with
`CHARTGEN_SERVER_ALERTS=1`.

- **Who.** `CHARTGEN_SERVER_ALERT_CLIENTS` is the operator's allowlist of client ids:
  only those clients can create alerts, and they get webhooks. **Unset, anyone may create
  alerts but nobody gets webhooks.** That is deliberate: an alert in a panel sends nothing
  anywhere and the limits below bound what it holds, while a webhook is a request made for
  a stranger, and reveals this server's address. `*` opens both, for an operator who
  accepts that. Taking a client off the list stops its alerts and webhooks at once.
- **Scope.** Price levels and crossings on Binance symbols. Drawing and indicator
  alerts need the browser's drawings and series and stay there, and the dialog says
  so. A stock has no feed, so the server refuses it.
- **Ownership and storage.** A row in `chartgen.db`, owned by the OAuth client.
  Clients that own alerts are not evicted or collected, as with scripts
  ([ADR-0025](/chartgen/decisions/0025-user-indicator-registry/)). The store exists
  only with the `scripting` feature, so a build without it has no server alerts.
- **Prices.** `ChartHub::watch_feed` joins the hub's feed for a symbol and interval
  and holds it with a lease. Live alerts on a symbol all read its 1m feed, whatever
  interval they are labelled with, so any interval works as a label. Alerts on bar closes
  read that interval's feed, which needs a stream of its own. No REST polling is added.
  The one fetch is a check of a symbol new to the server, on the path `generate_chart`
  uses, and the feed's own backfill after a reconnect: both go through the Binance gate in
  `src/binance_gate.rs` once that is merged. New feeds are limited to 40 a minute across
  charts and alerts, and to 96 connected at once with the lingering ones, because Binance
  bans an IP that connects too often, and that would stop every chart.
- **Closes.** A feed with trades rolls the candle on the first trade past the boundary, so
  the closing kline usually arrives after the next candle's first tick. An older bar is
  therefore a late close, read by close alerts only, unless it closed before or before the
  watch began. The stream's closed flag decides what closed, never the clock: a 28-day month
  has to count. The backfill stops sending the forming candle, which would look like a close.
- **Fire rules.** The browser's, ported as they are and pinned by one fixture both
  sides run (`tests/fixtures/alert_semantics.json`). A crossing needs a previous
  reading. That reading is in memory on purpose, so it is lost on a restart, and
  the first price after one is only a new baseline. A replayed bar (the backfill
  after a reconnect) or a lagged feed also starts the baseline over, as a gap does
  in the browser. One rule differs on purpose: a new message or webhook does not bring
  a spent alert back, because it would fire again with nothing about it changed.
- **No duplicate fire.** A fire writes the alert's new state and its delivery in
  one transaction before anything is sent. After a restart `once` is off, `every`
  greater / less is still disarmed, and a crossing has no previous reading.
  Unfinished deliveries resume under the same event id, for an hour.
- **No second evaluator.** Server alerts are not in the browser's list, so no tab
  can evaluate them. The leading tab polls `list_server_alerts` every 5 s and shares the
  list through storage; it reports a fire, and a count in shared storage keeps it to one
  report across tabs and reloads. While the tool is not listed it asks only `tools/list`,
  on focus or every 5 minutes, so the feature costs a server that has it off nothing.
- **Webhook reach.** https, port 443, no credentials in the URL. The name is
  resolved here and **every** answer must be a public address; the connection is
  pinned to those addresses, so a rebind changes nothing, and no proxy is used. IPv6
  must be global unicast, which also refuses mapped, compatible and NAT64 forms.
  6to4 and Teredo are refused too. Redirects are not followed. The response body is
  never read. Error texts are fixed strings, because a request error prints its URL,
  which may hold a token. Refused too: a name with no dot, private suffixes such as
  `.internal` and `.docker`, the hosts this server fetches from (Binance, Yahoo,
  alternative.me), its own `CHARTGEN_BASE_URL` host, and any prefix the operator lists in
  `CHARTGEN_WEBHOOK_BLOCKED_PREFIXES`. A name that does not resolve and one that resolves
  to a private address read the same, and `update` checks ownership first, so the answers
  say nothing about the network inside. A setting that does not parse keeps the feature off.
- **Delivery.** 5 s per attempt, 3 attempts, 2 s and 8 s apart. Retried on a timeout,
  connection failure, 5xx or 429 only. Each attempt resolves and checks again. A delivery
  over budget is not dropped: it waits in the store, and at each turn the client that waited
  longest goes first, at an even pace, so a backlog cannot keep another client's `once` alert
  waiting. In-flight slots are held per attempt, not through the wait between attempts.
- **Direct or relayed.** Direct by default, ignoring any proxy in the environment, because
  the connection is pinned. `CHARTGEN_WEBHOOK_PROXY` sends webhooks through a proxy instead,
  so the receiver sees the proxy's address. The check before sending still runs here, but the
  proxy resolves and connects, so it must be somewhere that cannot reach this network. If it
  cannot be used the delivery fails; it never falls back to direct.
- **Signing.** Optional HMAC-SHA256 over `<timestamp>.<body>` with a per-alert secret
  shown once. It is stored because the server needs it to sign, and is never returned
  or logged. A new URL does not keep it, because it signs for one receiver. A list shows a
  webhook's host, never its URL, whose path may be a token.
- **Limits.** 20 alerts per client, 200 not yet spent in total and 1000 rows, 16 symbol and
  interval pairs and 2 per client (`CHARTGEN_SERVER_ALERT_PAIRS_PER_CLIENT`), 10 creates or
  updates per client per minute, 6 fires per `every` alert per minute, 30 webhooks per client
  and 120 in total per minute, 50 deliveries waiting per client and 2000 in total, 16 in flight
  and 2 per client. A fire with a full queue still counts, and the alert says the webhook was
  skipped.
- **Expiry.** A spent alert is deleted 7 days after it fired. An alert is deleted when its
  owner has had no token for 30 days, unless the operator listed that client.
- **Eviction.** An owner is protected from OAuth client eviction only while the feature is
  on: with it off nobody runs those alerts, so its rows pin nobody, and an owner evicted then
  reads as idle when it comes back on.
- **Testing.** `CHARTGEN_WEBHOOK_TEST_ORIGIN` allows one loopback origin. It is read
  at startup and only counts with `CHARTGEN_DATA_SOURCE=sample`, so no request can
  set it and a live server ignores it.

### Consequences

- An alert fires with no browser open, and can notify another system.
- Delivery is at least once. A crash between the send and clearing the row sends the
  same event id again, so a receiver should deduplicate on it.
- A crossing that happens while the server is down is not reported, as with a closed
  tab. `greater` and `less` fire on the first price afterwards if they still hold.
- A crossing inside a feed's backfill after a reconnect can fire late, at most nine
  bars, because those closes are real and newer than the last one read.
- Each watched pair holds a WebSocket with the trade stream, even with no chart open.
- An alert can be deleted for nobody having used it: an unlisted owner with no token for 30
  days loses it. A `once` alert a person set and forgot for that long goes too. Listed clients
  keep theirs.
- With `*`, or with only the flag on, a stranger can still use up the alerts or the watched
  pairs until they expire, which refuses other people's new alerts, because client ids are free
  and the limits cannot attribute them. There is no operator command to list and delete alerts
  yet. That is why the allowlist is the recommendation.
- **One webhook reveals this server's IP.** A receiver sees where the request came from, so a
  server hidden behind a tunnel gives its home address to anyone it sends a webhook for. It is
  the first request here whose destination someone else chooses. The allowlist limits who can do
  it; `CHARTGEN_WEBHOOK_PROXY` hides the address from everyone, those clients included.
- A stranger with `*` can make this server send small POSTs to any other public HTTPS host,
  within the budgets. That is what a webhook is.
- **Not covered: public addresses inside this network.** The checks cannot tell that a public
  address is this network's own: its WAN address, or a LAN device's global IPv6 address, which
  the router's firewall does not guard on-link. TLS limits this: a webhook only delivers HTTP where
  a publicly trusted certificate matches the name, so mostly it is a connection and a handshake. An
  operator should list those addresses in `CHARTGEN_WEBHOOK_BLOCKED_PREFIXES`. Webhooks also come
  from this server's IP, so a rule elsewhere that trusts that IP, such as a Cloudflare Access bypass,
  would let them through.
- Server alerts have no line on the chart and are not in the Object Tree yet. Stocks
  are not supported.
