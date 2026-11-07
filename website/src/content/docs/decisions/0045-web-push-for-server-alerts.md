---
title: "0045 — Browser push for server alerts"
description: "A server alert can notify the user's browser or phone through Web Push with chartgen closed. Push is one more delivery next to the webhook, and only reaches known push services."
sidebar:
  order: 45
---

**Status:** Accepted
**Date:** 2026-11-06
**Amended by:** [ADR-0044](/chartgen/decisions/0044-sign-in-through-github-and-google/) (with sign-in on, devices belong to the user, the last outcome is kept per device, and an endpoint has one owner)
**Amends:** [ADR-0042](/chartgen/decisions/0042-server-side-alerts/) (a fire may also be delivered as a push, and push does not need webhook access)

## Context and Problem Statement

A server alert fires with no browser open
([ADR-0042](/chartgen/decisions/0042-server-side-alerts/)), but the only way it can tell anyone
then is a webhook, which needs a receiver of your own. The panel only reports it when a tab
opens. A user who closes chartgen wants this browser or phone to buzz.

## Decision Drivers

- Works on desktop browsers, Android, and iOS, without an app store.
- A stranger can sign in ([ADR-0036](/chartgen/decisions/0036-own-origin-invalid-client-redirect/)),
  so this must not let one point the server at a host of their choice.
- As reliable as a webhook: a restart neither loses a delivery nor fires again.
- No double report with a tab open.
- The server runs on a 4 GB Radxa, so the build should not get heavier.

## Considered Options

1. **Web Push, as one more delivery beside the webhook.** Reuses the row, queue, budgets, pinning
   and retries.
2. **A parallel push system.** Its own queue and retries: twice the code to get right, and two
   places for a limit to be missing.
3. **Email or a chat bot.** Needs an account with a third party, and an address from each user.

And for the crypto:

- **`openssl`, already linked through `native-tls`.** No new crate. RFC 8291 and the VAPID token
  are about 200 lines over its primitives, checked against the RFC's test vector.
- **`p256`, `aes-gcm` and `hkdf`.** Pure Rust, but more crates to build on the Radxa.
- **A dedicated Web Push crate.** Not tried: sending has to stay on the pinned path here, so it would
  only supply what those lines and the test vector do.

## Decision Outcome

Option 1, with `openssl`. An alert has `push: true` beside its optional webhook. A browser
registers its subscription with `subscribe_push`, per owner.

- **Delivery.** A fire writes its state and one row per delivery in one transaction: the webhook,
  and one per device. All of them carry one body with one event `id`. A restart resumes them.
  They wait in the owner's queue, with its cap and the hour's expiry, and go out through the
  webhook path's pinning, slots and retries (twice, after 2 s and 8 s, or as long as a push service asks in
  `Retry-After`, up to a minute; if it asks for longer, the delivery ends). Each message is encrypted
  under RFC 8291 (aes128gcm) with a new key and salt, and authorised with a VAPID token.
- **A dead subscription.** 404 or 410 from the push service deletes the device, with what it still
  had queued. Any other failure retries, then is recorded on the alert, and the device stays.
- **Where requests go.** An endpoint comes from the browser, so nothing but a check here says it
  points at a push service. It must be `https` on port 443 at `fcm.googleapis.com`,
  `updates.push.services.mozilla.com`, `*.push.apple.com` or `*.notify.windows.com`. The existing
  public-address checks and the pin apply as well. `CHARTGEN_WEBHOOK_PROXY` applies too. Legacy
  `android.googleapis.com` is left out, because a subscription made with a VAPID key does not use it.
- **What it reveals.** Requests go only to those hosts, so this channel does not reveal the home IP
  to a server the user chose, which a webhook does. **The push service itself still sees this
  server's IP**, unless the proxy is set. It cannot read the message.
- **Access.** Push goes with being allowed alerts at all: with `CHARTGEN_SERVER_ALERT_CLIENTS` unset,
  a stranger gets push where webhooks are refused, because there is no URL of their choosing. Keep
  the limits: 5 devices per client, 2000 in total, 30 pushes per client per minute, and 120 deliveries
  of either kind per minute across the server. A new device over the client's limit replaces its oldest,
  because a cleared site leaves a dead one behind that nobody can name.
- **A full table.** Client ids are free, so strangers can fill the 2000. A refusal would then lock everyone
  out for 30 days, so a new device replaces someone else's instead: never a listed client's, and first those
  whose owner has no active alert that notifies, the one idle longest among them. Going by idleness alone would
  let a flood push out real users, since junk clients hold fresh tokens. A key's text is limited too, before
  it is decoded, because the raw text is what is stored.
- **Owner.** A device has the same owner as an alert, the OAuth client of the token, and tool calls
  get it in one place (`server_alert_call`'s caller in `src/server.rs`). The same endpoint under two
  owners is two devices, so knowing an endpoint does not let anyone take or remove someone else's.
  Signing out does not remove a device, so alarms keep working; `unsubscribe_push` does, per device.
  **The web app keeps one OAuth client per browser**, so a phone only gets the alerts made on that phone,
  until sign-in moves `push_devices.owner` to users with the alerts. An iOS Home Screen app probably counts
  as its own browser, which has not been tried.
- **A leaked client id.** Whoever holds a client id can get a token for it
  ([ADR-0036](/chartgen/decisions/0036-own-origin-invalid-client-redirect/)), so push is as open as the rest of
  that client: they can replace its devices with their own, and then receive its fires and notes. Sign-in
  fixes this. Until then a listed id is a password
  ([ADR-0042](/chartgen/decisions/0042-server-side-alerts/)).
- **Keys.** `CHARTGEN_VAPID_PRIVATE_KEY`, or one made once and kept in the store. A subscription is
  tied to it, so changing it stops every device until its browser next opens chartgen, which
  subscribes again. Without `CHARTGEN_VAPID_SUBJECT` or an https `CHARTGEN_BASE_URL`, push stays off,
  because push services want a way to reach the operator.

### One report per fire

An alert that notifies this device has one voice, the service worker, which always shows its
notification. A browser may end the subscription of a site whose pushes show nothing, so it never
stays silent, even with a tab open. The tab says nothing about that fire and marks it as told. It only
speaks up if the push failed, and until the server knows the outcome it waits, so a tab that polls before
the push arrives does not report it too. An alert without push, or a device with push off, is reported by
the tab as before.

A push service accepting a message is not the device getting it, and it drops one for a device that stays off
longer than the hour the message lives. So a tab that first sees a fire more than an hour old reports it,
though the device may have shown it already: a second report beats a silent alarm. The outcome the tab reads is
the last one across the owner's devices, so with several devices it can also report twice, or stay quiet when
only this device failed. Recording it per device matters once sign-in puts several browsers under one owner.

### The web app

`/sw.js` and `/manifest.webmanifest` are served from the site root and, not being under `/assets/`,
with `no-cache` ([ADR-0027](/chartgen/decisions/0027-serve-web-app-same-origin/)), and a new worker takes
over as soon as it installs. The worker handles `push` and `notificationclick` and has no fetch handler, so
it caches nothing and cannot serve a stale app or stand between the page and `/mcp`. Clicking a
notification shows that symbol in an open tab, or opens one at `/?symbol=…`. The permission is asked
only from the enable button, whose states are unsupported, iOS not installed, blocked, off and on.
Nothing is registered until that click, so a server with alerts off costs nothing.

### Consequences

- Good, because a phone gets the alert with chartgen closed, and iOS gets it once the app is on the
  Home Screen, which iOS requires.
- Good, because push shares every limit and every test of the delivery path, and adds no crate.
- Good, because a stranger's push cannot be aimed at a host of their choice.
- Bad, because the server now sends requests to Google, Mozilla, Apple and Microsoft, who see its IP.
- Bad, because `openssl` is now a direct dependency, where `native-tls` only needed it on Linux. On macOS it is
  vendored, so the published binaries do not need Homebrew's and their build compiles OpenSSL. A Windows
  build, which is not published, would need it installed.
- Bad, because a delivery still gives up after its retries, as a webhook's does: a push service that is
  down for a minute loses that fire's notification, and the alert says so.
- Bad, because a client's deliveries stay in order across kinds: its webhook over budget holds back
  its own pushes behind it, and a silent webhook host holds its slots.
- Bad, because only an open page can tell the server that a browser replaced its subscription
  (`pushsubscriptionchange` has no token), so that device is silent until chartgen is next opened.
- Neutral: alerts whose owner had no token for 30 days are still deleted
  ([ADR-0042](/chartgen/decisions/0042-server-side-alerts/)), and devices with them, unless the
  operator listed the client. Someone relying on push who never opens chartgen loses them then.

### What could not be tested

Tests use a fake push service, through `CHARTGEN_WEBHOOK_TEST_ORIGIN`, which a request cannot set and
which is ignored on a live source. Real delivery through FCM, Mozilla, Apple or Windows, and a real phone,
have not been tried; [Enabling push](/chartgen/guides/deploy/#browser-push) lists how to check them.
