---
title: "0050 — A signed-in user's workspace, on any device"
description: "Settings, layouts, watchlists, drawings and browser alerts follow the account: localStorage stays the working copy per account, a per-user document store holds them, and devices merge three-way by item through one endpoint."
sidebar:
  order: 50
---

**Status:** Accepted
**Date:** 2026-11-07
**Supersedes in part:** [ADR-0044](/chartgen/decisions/0044-sign-in-through-github-and-google/) (browser data now follows the account, and is no longer left for the next person on a browser)

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
| All users | 1 GiB, the last tenth only for accounts already holding documents | `quota` for every write that grows |
| Deleted documents | 90 days and the newest 1000 per user, outside the document limit | Older ones forgotten |
| One request | 50 writes, 1 MiB | `400` / `413` |
| Requests | 120 a minute and 8 MiB a minute per user | `429` with `Retry-After: 60` |
| At once | 1 per user, 8 in all, refused rather than queued | `429` with `Retry-After: 1` |

Only the keys the app syncs are stored, in a small alphabet, at most 128 long. A refusal of one key
affects that key: the rest of the request goes through, and existing rows stay. Deleting a document keeps a
tombstone, so other devices learn of it. A device with no cursor is told of none, as it has nothing
to delete. The user comes from the token only: nothing
in a request can name another.

A write carries the version it was based on. If that is not the current one, nothing changes, and
the answer holds the current document to merge with. The same value sent twice is a success, so a
device that lost the answer to a request can retry.

Anyone can sign in, so what a request may cost matters as much as what an account may keep. The token
and the limits are checked before the body is read, so a caller who may not send one costs nothing,
and a body must arrive within 10 s. A running sync keeps its place until its answer is written out,
because an answer a client does not read is memory too, up to the size of what that account holds.
Its share of the 8 ends after 30 s even so, so that clients that stop reading cannot keep everyone
else out for long; the one per user stays until the connection goes. To a device all of these are a
pause: what it has stays, and goes out when the `Retry-After` has passed.

In a 256 MiB container, in a debug build, the server stayed under 97 MB with ten accounts of 2 MiB each
sending 100 parallel reads, 100 parallel 0.95 MiB writes, and 200 requests whose answers they did not
read. Before these limits, two accounts took it to 486 MB.

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
| A new token, answered with no user | Live keys hidden and an error with **Sign in again**. Going through the sign-in again would come back here, without end |
| No answer, no new token | The data as it was left: the app works with the server away |
| No answer, a new token, data from a sign-in here | Live keys hidden and an error: this may be someone else |
| No answer, a new token, nothing from a sign-in here | As before: no account's data is here to show, and a server without sign-in keeps working |

Signing out sends what is pending, ends the session, and hides the live keys. The marker stays, so a
stray write from a tab on its way out is still known to be that account's. A scoped copy always
beats a stray live value of the same key: without this, a store's defaults could replace an
account's data. A running tab reloads when the marker changes under it, including where the server has no store and
nothing syncs: the keys are still one account's. What it holds back for the moment the page goes (a drawing's
last change, a script draft) it then drops, rather than write it into the new account's keys.

Scoped copies remain in the browser's storage. Another person using the devtools on a shared browser
can read them, as they could read any site's `localStorage`. They are out of sight for the app, not
encrypted. Clearing site data removes them, and what has synced is still on the server.

### In the browser: what syncs

Synced: `chartgen.layouts`, `chartgen.layout.*` (including the grid), `chartgen.pane.*`,
`chartgen.indicators.active`, `chartgen.watchlist.*`, `chartgen.drawings.<SYMBOL>`,
`chartgen.alerts.list`, `chartgen.chartSettings`, `chartgen.orderflow`. Not synced: the backup of an unreadable layouts
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
that tab would save from memory in the same way. **Reload** reloads all tabs of the browser. On plain
http off localhost browsers have no Web Locks, so boot applies it only where no tab can be running: on
the first load of the account in this browser, or within 30 s of a **Reload**.

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

A save of a symbol's drawings is itself merged against what is stored, with what that page last loaded
or saved as the base. Another tab on the symbol, which holds its own copy in memory, so takes away only
what it saw and removed. It shows the other tab's drawings only after a reload.

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
are united. The yes is recorded and carried out by the next boot, after sync and before any store
loads, so a store saving from memory cannot cost the old data; every tab reloads for it. Any later account is not offered it and does not see it. If sign-in is later turned off,
the data comes back.

### Consequences

- Good: a device new to an account finds everything, with no browser having been connected since the
  server restarted: the server holds the documents.
- Good: `/workspace/sync` adds no setting and no schema to `tools/list`.
- Good: one account's flood stops at its own limits, or at the total, and a row of a deleted document is
  a key and a version: about 200 KB per account at most.
- Bad: the 8 syncs at once can all be taken, by 8 accounts at a time. Then others get `429` and wait, and
  their work stays in their browsers.
- Bad: an answer a client does not read holds memory until it hangs up, one per account: accounts with
  2 MiB each that do so can in principle add up, at about 2.5 MiB per account.
- Bad: a stranger with many accounts can fill the 1 GiB total, 512 accounts of 2 MiB. New accounts stop
  at nine tenths, so accounts already here go on for a while; past the total nobody's documents grow,
  existing ones stay, and everyone keeps working locally with a notice. It is cleared by deleting rows
  ([persistence](/chartgen/reference/persistence/#chartgendb-oauth-clients-scripted-indicators-and-server-alerts)).
- Bad: restoring `chartgen.db` from a backup rolls the account back. The server says so when a device's
  cursor is past its versions, and the device takes the file's documents as the current ones, and sends what the file
  does not have as new. What changed since the backup in a document the file does have is replaced at
  the next reload, or merged if it was edited since the last round.
- Bad: a device away for more than 90 days, or for over 1000 deletions, can bring back something deleted
  since, because the tombstone is gone. Its write looks like a new document.
- Bad: two tabs on a document other than drawings. Layouts, watchlists and browser alerts follow other
  tabs through `storage` events, so that case is narrow. A setting is one value, and the later save
  wins, as it always has. Now that goes to every device too.
- Bad: mixed builds. An older build that rewrites a layout may drop fields it does not know, for the
  items it rewrites. A layouts file it cannot read at all is backed up and not written on load
  ([#111](https://github.com/5queezer/chartgen/pull/111)), but saving later replaces it.
- Bad: each document costs up to three copies in `localStorage` (live, base, server's), so an account
  near its 2 MiB limit takes most of a browser's quota. If the browser's storage fills up, sync stops
  with a notice, and local work goes on as far as the stores can save.
- Bad: two devices new to an account within a few seconds, before either has sent anything, each
  create a default layout, and the account keeps both. Deleting one is an ordinary edit.
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
