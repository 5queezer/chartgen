---
title: "0050 — A signed-in user's workspace, on any device"
description: "Settings, layouts, watchlists, drawings and browser alerts follow the account: localStorage stays the working copy per account, a per-user document store holds them, and devices merge three-way by item through one endpoint."
sidebar:
  order: 50
---

**Status:** Accepted
**Date:** 2026-11-07
**Amends:** [ADR-0044](/chartgen/decisions/0044-sign-in-through-github-and-google/) (browser data now follows the account, and is no longer left for the next person on a browser)

## Context and Problem Statement

With sign-in ([ADR-0044](/chartgen/decisions/0044-sign-in-through-github-and-google/)), scripts and
server alerts follow an account, but chart settings, layouts, grids, watchlists, indicators,
drawings and browser alerts stayed in one browser's `localStorage`, visible to whoever signs in
next on it. A user should find the same workspace on any browser or device. With no sign-in
configured, or for a browser never signed in, the app must behave exactly as before.

Anyone with a GitHub or Google account can sign in, so this data is written by strangers: it needs
limits, and no path may read or overwrite another account's.

## Decision Drivers

- No path to another account's data, on the server or in a shared browser.
- The app stays fast and works with the server away: nothing waits on sync while the app runs.
- Two devices editing must not wipe each other, and what is lost in a conflict is said, not silent.
- The stores keep working as they are: about a dozen modules read `localStorage` as they load and
  save from memory.
- Documents from a newer build must survive an older one
  ([#111](https://github.com/5queezer/chartgen/pull/111)).

## Considered Options

**Storage:**

1. One table of documents per user, keyed by (user, key), with a version.
2. A table per kind (layouts, drawings, alerts, ...).
3. Files per user.

**Transport:**

1. One REST endpoint for the web app.
2. MCP tools, like the rest of the app's data.

**Sync model:**

1. Per-document version check, with a three-way merge by item in the browser.
2. Last writer wins on whole documents.
3. An operation log or CRDT per collection.

## Decision Outcome

**Storage: option 1.** The server does not need to understand the documents, and a new kind of
setting needs no migration. `workspace_docs` holds (user, key, value, version). A per-user sequence
in `workspace_users` gives each write a version, so "what changed since" is a single indexed read.

**Transport: option 1.** `POST /workspace/sync` takes `{since, writes}` and answers what became of
each write and what changed since. It needs conditional results per key and has no use for an
agent. It serves only the web app's own token: one whose code went to this server's own root
(`is_first_party`). An application the user approved at the consent page, though it acts for the
same user, gets `403`: its grant is to the tools, not to what is in the browser.

**Sync model: option 1.** Whole-document last-writer-wins would let one device's drawings erase
another's. An operation log would need every store rewritten. The stores save whole documents, so
the difference between what they last saw and what they hold now is what the user did.

### On the server

| Limit | Value | When reached |
|---|---|---|
| One document | 512 KiB | `too_large` for that key |
| One user | 2 MiB and 1000 documents | `quota` for that key |
| All users | 1 GiB | `quota` for every write that grows |
| One request | 50 writes, and the body limit | `400` / `413` |
| Requests | 120 a minute per user | `429` with `Retry-After` |

Keys must start with `chartgen.` and use a small alphabet, at most 128 long. A refusal affects one
key: the rest of the request goes through, and existing rows stay. Deleting a document keeps a
tombstone, so other devices learn of it, for 90 days. The user comes from the token only: nothing
in a request can name another.

A write carries the version it was based on. If that is not the current one, nothing changes, and
the answer holds the current document to merge with. The same value sent twice is a success, so a
device that lost the answer to a request can retry.

### In the browser: accounts

Everything under `chartgen.` belongs to the account, except the browser's own keys:
`chartgen.oauth.*`, `chartgen.push.*` ([one owner per endpoint](/chartgen/decisions/0044-sign-in-through-github-and-google/)),
and the keys of this mechanism. `chartgen.workspace.account` says whose the live keys are. Others
wait under `chartgen.u.<account>|<key>`.

`main.tsx` now waits for `bootWorkspace` before importing the app, because the stores read
`localStorage` as they are imported. So no store ever loads another account's data, and nothing
flashes on screen. Boot consumes the OAuth redirect, asks `/oauth/me`, and then:

| Answer | What happens |
|---|---|
| Sign-in off, nothing from one here | Nothing is written or moved: exactly as before |
| Sign-in off, data from one here | Live keys go to that account's scope, older data comes back |
| A user | That account's keys become live, first sync, then the app |
| Sign-in on, no user | Off to the sign-in, with the address kept in `sessionStorage`. Nothing is hidden, since nothing renders and another tab may be running on this data |
| Sign-in on, no user, an error to show | Live keys hidden, then the app with the error |
| No answer, no new token | The data as it was left: the app works with the server away |
| No answer, a new token | Live keys hidden and an error: this may be someone else |

Signing out sends what is pending, ends the session, and hides the live keys. The marker stays, so a
stray write from a tab on its way out is still known to be that account's. A scoped copy always
beats a stray live value of the same key: without this, a store's defaults could replace an
account's data. A running tab reloads when the marker changes under it.

Scoped copies remain in the browser's storage. Another person using the devtools on a shared browser
can read them, as they could read any site's `localStorage`. They are out of sight for the app, not
encrypted. Clearing site data removes them, and what has synced is still on the server.

### In the browser: what syncs

Synced: `chartgen.layouts`, `chartgen.layout.*` (including the grid), `chartgen.pane.*`,
`chartgen.indicators.active`, `chartgen.watchlist.*`, `chartgen.drawings.<SYMBOL>`,
`chartgen.alerts.list`, `chartgen.chartSettings`. Not synced: the backup of an unreadable layouts
file, the drawings LRU index, the sidebar tab, the last drawing tool, script drafts (private, and
per device), the cached server alerts, which come from the server anyway.

Each document has a state in `chartgen.sync.<key>`: the server's version, `b` (the base: what the
stores last saw) and `s` (what the server holds, if different). A background round scans every 5 s,
and posts when something changed, or every 30 s while visible:

- Live equals `b`: nothing to send.
- Otherwise merge `b`, live and `s`. If the result is what the server already holds, `b` catches up
  and nothing is sent. If not, the result goes out with the version.
- On `ok`, `b` becomes what the stores had, `s` what was sent. On `conflict`, `s` becomes the current
  document, and the next round merges with it.

**Nothing from another device is applied under running stores.** They save from memory, so a value
placed under them is overwritten at their next save, and that save would look like the user removing
it. What arrives waits in `s`, and the banner offers **Reload**. Boot applies it (`settle`) before
any store loads, unless another tab of the app is running, which a shared Web Lock tells, because
that tab would save from memory in the same way. **Reload** reloads all tabs of the browser.

A round stops if the marker is another account's or any of its account's keys are in a scope. Keys
are copied before they are removed, so a tab that sees a key gone also sees the scope in use, and
does not take a move for a deletion.

### Merge, and what is lost in a conflict

Layouts (`items`), watchlists v2 (`lists`), browser alerts and each symbol's drawings are collections
by `id`. Each item is merged against the base, and other top-level fields the same way. Items keep
their text, so fields only a newer build knows come through whole. Anything else is one value, as is
a collection whose shape this build cannot read: nothing is garbled by trying.

| Base → here / other | Result | Lost, and said |
|---|---|---|
| One side changed, or deleted | That side | Nothing |
| Both added different items | Both | Nothing |
| Both edited one item or value | This device, the later writer | The other device's edit, with a notice on this device |
| One edited, one deleted | The edit | The deletion, with a notice |
| New value on a device with no base, against the account's | The account's | This device's value, with a notice |
| Collections with no base | United | Nothing |
| Both changed which entry is active | This device | Nothing worth saying: it is a selection |

A notice names the document ("your drawings on BTCUSDT"), not the item. The replaced version is not
kept. Two devices adding symbols to one watchlist are both editing that list, so one list wins, with
a notice: the items of a list are not merged.

Dropping a symbol's drawings from the local LRU is not a deletion: the state marks it, nothing is
sent, and they return at the next load. For signed-in users this means the LRU no longer frees room
for what the account holds.

Refused documents (`too_large`, `quota`, `bad_key`) stay in the browser, are named in a notice, and
are not sent again until they change.

### Data from before any sign-in

The first sign-in on a browser moves existing data aside (`chartgen.legacy|<key>`), in both
directions: nothing goes to the account and nothing replaces it without a yes. The first account to
sign in on that browser is offered it, with **Add to my account**, **Delete them** and **Not now**,
and the offer returns until decided. Adding only adds: a value the account has stays, and collections
are united. Any later account is not offered it and does not see it. If sign-in is later turned off,
the data comes back.

### Consequences

- Good: a device new to an account finds everything, with no browser having been connected since the
  server restarted: the server holds the documents.
- Good: `/workspace/sync` adds no setting and no schema to `tools/list`.
- Good: one account's flood stops at its own limits, or at the total.
- Bad: a stranger with many accounts can fill the 1 GiB total. Then nobody's documents grow, existing
  ones stay, and everyone keeps working locally with a notice. It is cleared by deleting rows
  ([persistence](/chartgen/reference/persistence/#chartgendb-oauth-clients-scripted-indicators-and-server-alerts)).
- Bad: a device away for more than 90 days can bring back something deleted since, because the
  tombstone is gone. Its write looks like a new document.
- Bad: a browser with two tabs on one document saves from each tab's memory, which is how it has
  always behaved, and now the later save can take away what the other tab added, for all devices.
  Drawings on one symbol in two tabs is the case to think of.
- Bad: mixed builds. An older build that rewrites a layout may drop fields it does not know, for the
  items it rewrites. A layouts file it cannot read at all is backed up and not written on load
  ([#111](https://github.com/5queezer/chartgen/pull/111)), but saving later replaces it.
- Bad: each document costs up to three copies in `localStorage` (live, base, server's), so an account
  near its 2 MiB limit takes most of a browser's quota. If the browser's storage fills up, sync stops
  with a notice, and local work goes on as far as the stores can save.
- Bad: browser alerts are evaluated by each browser on its own, as before. Two devices with the app
  open both notify, and both updating an alert's state can show a conflict notice.
- Neutral: the symbol and timeframe sync like any setting, so a reload shows what the last device
  chose.

### Server-side drawing and indicator alerts

Drawings and indicator settings now live on the server, so they could be evaluated with no browser
open. That is a separate step, and needs:

1. A parser for drawing records in Rust, tolerant in the way the browser's is. The documents are opaque
   to the server today.
2. Drawing geometry over time, in Rust (trend lines, rays, channels), matching
   `web/src/lib/drawingMath.ts` and its tests.
3. Evaluating indicators server-side for each alert's symbol, timeframe and parameters, on the load
   budget of [ADR-0049](/chartgen/decisions/0049-one-kline-read-per-symbol/).
4. Moving ownership of those alerts from a synced document to `server_alerts`, so they have delivery,
   a device and a limit, and so a browser no longer fires the same alert too.

Reading another user's document for it stays impossible: an alert runs on its owner's.

## Testing

- Store: `src/scripting/tests.rs`, conflicts, versions, tombstones, limits, a restored file, and the
  upgrade from a file before this migration.
- Real binary: `tests/e2e.rs`, `test_workspace_*`: two users, several clients, a restart with no
  browser connected, the third-party refusal, limits.
- Merge and scoping: `web/src/lib/workspaceSync.test.ts`.
- Real browsers: `web/tests/e2e/signin-workspace.spec.ts`, where each context is a device, and
  `workspace-off.spec.ts`, where sign-in is off and no trace appears.
