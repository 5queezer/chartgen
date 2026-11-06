---
title: "0042 — Alerts that run on the server, with webhooks"
description: "A price alert can run on the server with no browser open and notify a webhook. It reads the shared Binance feeds, follows the browser's fire rules, and its webhook can only reach public HTTPS addresses."
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

- **Scope.** Price levels and crossings on Binance symbols. Drawing and indicator
  alerts need the browser's drawings and series and stay there, and the dialog says
  so. A stock has no feed, so the server refuses it.
- **Ownership and storage.** A row in `chartgen.db`, owned by the OAuth client.
  Clients that own alerts are not evicted or collected, as with scripts
  ([ADR-0025](/chartgen/decisions/0025-user-indicator-registry/)). The store exists
  only with the `scripting` feature, so a build without it has no server alerts.
- **Prices.** `ChartHub::watch_feed` joins the hub's feed for a symbol and interval
  and holds it with a lease. Live alerts on a symbol all read its 1m feed, whatever
  interval they are labelled with. Alerts on bar closes read that interval's feed.
  No REST polling is added. The one fetch is a check of a symbol new to the server,
  on the path `generate_chart` uses, and the feed's own backfill after a reconnect:
  both go through the Binance gate in `src/binance_gate.rs` once that is merged.
- **Fire rules.** The browser's, ported as they are and pinned by one fixture both
  sides run (`tests/fixtures/alert_semantics.json`). A crossing needs a previous
  reading. That reading is in memory on purpose, so it is lost on a restart, and
  the first price after one is only a new baseline. A replayed bar (the backfill
  after a reconnect) or a lagged feed also starts the baseline over, as a gap does
  in the browser.
- **No duplicate fire.** A fire writes the alert's new state and its delivery in
  one transaction before anything is sent. After a restart `once` is off, `every`
  greater / less is still disarmed, and a crossing has no previous reading.
  Unfinished deliveries resume under the same event id, for an hour.
- **No second evaluator.** Server alerts are not in the browser's list, so no tab
  can evaluate them. Tabs poll `list_server_alerts`; the leading tab reports a fire,
  and a count in shared storage keeps it to one report across tabs and reloads.
- **Webhook reach.** https, port 443, no credentials in the URL. The name is
  resolved here and **every** answer must be a public address; the connection is
  pinned to those addresses, so a rebind changes nothing, and no proxy is used. IPv6
  must be global unicast, which also refuses mapped, compatible and NAT64 forms.
  6to4 and Teredo are refused too. Redirects are not followed. The response body is
  never read. Error texts are fixed strings, because a request error prints its URL,
  which may hold a token.
- **Delivery.** 5 s per attempt, 3 attempts, 2 s and 8 s apart. Retried on a timeout,
  connection failure, 5xx or 429 only. Each attempt resolves and checks again.
- **Signing.** Optional HMAC-SHA256 over `<timestamp>.<body>` with a per-alert secret
  shown once. It is stored because the server needs it to sign, and is never returned
  or logged. A list shows a webhook's host, never its URL, whose path may be a token.
- **Limits.** 20 alerts per client, 200 in total, 16 symbol and interval pairs,
  10 creates or updates per client per minute, 6 fires per `every` alert per minute,
  30 webhooks per client and 120 in total per minute, 500 pending deliveries, 16 in
  flight and 2 per client. A fire over a webhook budget still counts, and the alert
  says the webhook was skipped.
- **Testing.** `CHARTGEN_WEBHOOK_TEST_ORIGIN` allows one loopback origin. It is read
  at startup and only counts with `CHARTGEN_DATA_SOURCE=sample`, so no request can
  set it and a live server ignores it.

### Consequences

- An alert fires with no browser open, and can notify another system.
- Delivery is at least once. A crash between the send and clearing the row sends the
  same event id again, so a receiver should deduplicate on it.
- A crossing that happens while the server is down is not reported, as with a closed
  tab. `greater` and `less` fire on the first price afterwards if they still hold.
- A crossing inside a feed's backfill after a reconnect can fire late, at most ten
  bars, because those closes are real and newer than the last one read.
- Each watched pair holds a WebSocket with the trade stream, even with no chart open.
- Alerts do not expire. An abandoned client's alerts keep counting against the total
  until they are deleted by hand.
- A stranger can use up the total or the watched pairs, which then refuses other
  people's new alerts. Registration being open, the limits bound the damage but do
  not attribute it.
- A stranger can make this server send small POSTs to any public HTTPS host, within
  the budgets. That is what a webhook is, and it is why the default is off.
- **Not covered:** the checks cannot tell that a public address is this network's own
  WAN address. A webhook at it reaches whatever the router forwards on port 443, so
  an operator should forward nothing there, or keep server alerts off.
- Server alerts have no line on the chart and are not in the Object Tree yet. Stocks
  are not supported.
