---
title: "0058 — Refresh tokens, so a client authorized once keeps working"
description: "/token answers a refresh-token grant, and the access token of a client that holds a refresh token is known again after a restart. Both are stored only as hashes, so a headless MCP client needs a person once. The web app's own client gets neither, and POST /revoke ends them."
sidebar:
  order: 58
---

**Status:** Accepted
**Date:** 2026-11-11
**Amends:** [ADR-0044](/chartgen/decisions/0044-sign-in-through-github-and-google/) (a client other than the web app no longer re-authorizes after a restart or after 7 days, and its access token no longer ends at a restart; a refresh token counts as a token in the per-user limits), [ADR-0036](/chartgen/decisions/0036-own-origin-invalid-client-redirect/) (a refresh token keeps its client from being evicted, for up to 30 days out of use, and through restarts)

## Context and Problem Statement

Access tokens live in the server's memory and last 7 days, and `/token` had one grant,
`authorization_code`. Every restart ended every token, and a deploy is a restart
([ADR-0037](/chartgen/decisions/0037-pull-based-deploy/)). A browser recovers by itself: it
is sent through `/authorize` again and, with sign-in on, still has its session. A headless
MCP client, such as an agent that runs on a server and wakes on an event, has no browser and
nobody watching, so each restart and each seventh day needed a person. Such clients used the
stdio binary instead, which has no server alerts.

Registered clients, users, sessions and approvals already survive a restart in
`CHARTGEN_DB`. What a client holds to prove its authorization did not.

## Decision Drivers

- Authorized once, then no person: across a restart, a deploy and the 7 days of an access
  token.
- Only hashes of tokens on disk, and a revocation that still holds after a restart.
- The first-party rule of [ADR-0050](/chartgen/decisions/0050-per-user-workspace-sync/)
  stays: only the web app's own token reaches `/workspace/sync`, and nothing durable may
  become such a token.
- [ADR-0044](/chartgen/decisions/0044-sign-in-through-github-and-google/) stays: with
  sign-in on a token's owner is its user and never its client, and rows from before sign-in
  go to `CHARTGEN_CLAIM_OWNER` and to nobody else.
- The limits per user keep holding, and no request writes to the store.
- A standard client needs no code written for chartgen.

## Considered Options

**1. Store access tokens.** Keep them as hashes in the file, so a restart is invisible to
every client.

- Good: the smallest change, and no client has to do anything.
- Bad: the 7 days remain, so a person is still needed every week. Making the lifetime long
  or sliding turns a bearer token that travels on every request into a long-lived secret,
  and a sliding one needs a write per request.
- Bad: the web app takes a token per page load, so every page load would write, and its
  tokens end with a browser session, so signing out would have to delete rows.

**2. A refresh-token grant (chosen).** `/token` hands out a refresh token with the access
token and exchanges it for the next pair
([RFC 6749 section 6](https://www.rfc-editor.org/rfc/rfc6749#section-6)).

- Good: it is what MCP clients already implement for a token that has run out.
- Good: the long-lived secret goes to `/token` only, never on a tool call, and the store is
  written when a token is issued or exchanged, never per request.
- Good: the first-party rule is kept by construction, because a refresh token is never
  given to the web app's client and carries no browser session.
- Bad: the client must store its refresh token, and the first authorization still takes a
  browser, once.
- Bad: a rotating token can be lost between the server's answer and the client's disk.
- Bad: **by itself it does not survive a restart with the clients that exist.** A restart
  ends the access token while the client still believes it valid, the call gets `401`, and
  the common client answers a `401` with a new authorization, not with a refresh. See below.

**3. Personal access tokens.** A signed-in user creates a named, long-lived token in the web
app and pastes it into the agent's configuration.

- Good: no OAuth flow on the headless side at all, and a list in the web app is a natural
  place to revoke one.
- Bad: it needs a page in the web app and endpoints for it, and a second kind of credential
  outside clients, consent and the limits per client.
- Bad: a static secret that never rotates, in a configuration file and on every request.
- Bad: it does not exist with sign-in off, where there is no signed-in user.

## Decision Outcome

Chosen: **a mix of options 2 and 1. A refresh-token grant, and for the clients that get a
refresh token, the hash of their current access token is stored too,** so that a restart
does not end it. The refresh token removes the 7 days. The stored access-token hash removes
the restart. Neither puts a long-lived secret on every request, and neither needs a page in
the web app or a new kind of credential. Personal access tokens stay possible later, and
nothing here is in their way.

**Why the grant alone was not enough.** The first version kept access tokens in memory only,
as before, and relied on the client to refresh after the `401` that follows a restart. It
passed its own tests and failed with a real client. The official Python MCP SDK, `mcp`
1.26.0, refreshes only when its own clock says the access token has run out
(`mcp/client/auth/oauth2.py`, lines 491 to 498: `if not self.context.is_token_valid() and
self.context.can_refresh_token()`). On a `401` for a token it believes valid it runs the
whole authorization again (the same file, from line 505: `if response.status_code == 401`,
"Perform full OAuth flow"), which opens a browser. Run by hand against a local server, with
sign-in off: authorized once, a second client process needed nobody, and after a restart of
chartgen the client asked for a person again, with `/authorize` and a code exchange in the
server log and no refresh. With the access-token hash stored, the same three runs need a
person once, and after the restart the server log shows no `/authorize`, `/register` or
`/token` request at all.

Hermes, the first agent this is for, runs on MCP SDK 2.3.0 with its own token handling. It
stores the absolute time at which a token runs out and refreshes first when that has
passed (`mcp_oauth.py`, lines 468 to 495; `mcp_oauth_manager.py`, lines 80 to 89). After a
`401` it reloads its tokens from disk and otherwise leaves the refresh to the SDK. Run
against a local server with sign-in off, as reported by the operator: authorized once, a
tool call after a restart of chartgen succeeded with the same access token and no
`/authorize` or `/token` request, and after a restart of Hermes with a stored token that had
run out it refreshed by itself, both tokens rotated. A real 7 days, a spent token coming
back and sign-in on were not part of that run.

Rejected: **storing every access token**, option 1 as it stands. The web app's are tied to a
browser session and come one per page load. Rejected: **telling clients apart by a
`WWW-Authenticate` hint** such as `error="invalid_token"`: it is the standard signal, and
the client above does not act on it.

### What is issued, and to whom

The code exchange at `/token` answers with a `refresh_token` next to the access token,
unless the code went to the web app's own client: a `redirect_uri` that is exactly `/` on
the server's own origin (`is_first_party`, as in
[ADR-0050](/chartgen/decisions/0050-per-user-workspace-sync/)). That holds with sign-in on
and off. The web app takes a token per page load, its tokens must end with its browser
session, and it recovers through `/authorize` by itself, so for it nothing changes.

`POST /token` with `grant_type=refresh_token`, `refresh_token` and `client_id` answers with
a new access token, 7 days as before, and the next refresh token.

**A code is exchanged once.** One authorization is one family, which relies on a code that
cannot be exchanged twice. With sign-in on that was already so. With sign-in off a code
could be exchanged again and again for its 5 minutes, each time now for another refresh
token. It is used up by its first exchange in both modes, successful or not.

A refresh token is `<family part>.<secret>`, each 256 random bits. The family part is the
same in every token of one authorization, and is what lets a token the server has no row
for any more still end its family (below). The server keeps the SHA-256 of the whole token
and the SHA-256 of the family part, as the family's id, and nothing else, in memory and in
the new table `refresh_tokens`. A client treats the value as opaque. The value is in the answer to the client and
nowhere after that, including the log. "Only hashes are stored" is about what is on disk:
in memory, access tokens are still held by their value, as before.

A refresh token belongs to the client it was issued to and to the user who authorized, or
to no user with sign-in off. Another `client_id` is refused. The access token it yields has
the same owner as the first one had, and never a browser session, so it cannot pass for the
web app's and gets `403` from `/workspace/sync`.

### An access token after a restart

The row of a family's usable refresh token also holds the SHA-256 of the access token that
was issued with it, in a column that a second migration adds to the table. It is written in the same transaction, at the code exchange and at each
refresh, and never by a request that uses the token. A spent row loses it.

At startup these hashes are loaded. The first request that carries a matching token makes
it a token like any other again, under the value the client sent. Event streams and chart
subscriptions are keyed by that value, taken from the request, so they need nothing from
the store.

- **The 7 days are absolute.** They count from the time in the row, when the token was
  issued. A restart adds nothing, however many there are.
- **Only the newest access token of a family comes back.** The one before it, which has
  60 seconds after a refresh, ends at the restart.
- **It is never the web app's.** The web app's client has no family, so none of its tokens
  is stored, and a token that comes back has no browser session: `/workspace/sync` answers
  it `403`, as before the restart.
- It comes back only under the sign-in mode it was issued in, and only while its client is
  registered.
- It counts in the limits as before the restart: one access token of its client.

The web app's own tokens, and those of a client that got no refresh token, still end at a
restart.

### Rotation, and a token that comes back

One authorization is a **family**: its refresh tokens follow each other, and exactly one is
usable. Each exchange spends the presented token and stores its successor. The spent hash
is kept, so that a token that comes back is recognised, also after a restart:

- A spent token presented again **ends the family**, with the access tokens that came from
  it. Either the client or somebody with a copy holds the usable one, and the server cannot
  tell which, so neither keeps it.
- Except within 60 seconds, for the token exchanged last: a client that lost the answer may
  send the same token again, and again if that answer is lost too. Each retry gives a new
  successor and **replaces the one before it**, so the family has one usable token, never
  two. **A replaced successor is marked as such and is never itself good for a retry: any
  use of one ends the family**, at once, however soon it comes and whatever happened in
  between. So if a replaced successor had in fact reached somebody, they are found out the
  first time they use it. The mark is in the row and survives a restart. Only the newest
  replaced successor keeps its row. An earlier one has none and ends the family all the
  same, through its family part, like a token older than the ones that are kept. So
  retries do not make a family grow: it holds at most 4 tokens, 2 spent, one replaced
  and the usable one.
  Rejected: answering the retry with a second successor while the first stays valid. That
  leaves two lines of tokens that never meet, and a copy made in that window is never
  noticed. Rejected as well, after a first version did it: telling a retry by position and
  time alone. A replaced successor has the same position as the one that replaced it, so
  within a minute it passed for a retry and two holders could go on replacing each other.
- The 2 most recent spent tokens of a family are kept, which bounds the table. **An older
  one ends the family all the same.** The server has no row for it, but its family part
  names the family, and a token that names a family in use, comes from that family's
  client and is not its usable token has come back. Without this, the limit switched
  detection off: whoever held a copy could exchange it until the owner's token had no
  row, after which that token was merely unknown, the owner authorized anew and the copy
  lived on. Since detection does not rest on the rows, only the token exchanged last has
  to be kept, for the retry; the second is margin. A first version kept 100, which
  with sign-in off allowed about 2 million rows (1000 clients, 20 families each) in
  memory and in the file. With 4 per family it is about 80,000. A file written with 100
  is read as it is and loses the surplus at each family's next exchange.

**Who can end a family this way:** anyone who holds or ever held one of its tokens, and
presents it with the family's client id. That is the point of it. Nobody else: a value
that names no family in use ends nothing, a token of another client's family ends nothing,
and the database does not help, because it holds the hash of the family part and of each
token, and neither can be turned back into a value to present.

**The successor is in the store before the answer leaves.** The spent mark and the new row
are one transaction. If it fails, `/token` answers `503`, issues nothing, and the presented
token is still the usable one. Answered first, a failed write would leave the client with a
token that the next start does not know, which is the failure this decision removes. The
first refresh token of a family is treated the same way: if it cannot be stored, the answer
carries the access token alone.

**A request is finished even if its client hangs up.** A request that is dropped while it
waits would otherwise stop between the store write and the change in memory that follows:
the token spent in the store and still usable in memory, so that the same token again is
refused with `503` until a restart, or a family revoked in the store and still working. So
a refresh, a revocation and the storing of a new family run in a task of their own, which
the request only waits for. The client that hung up never got the answer, and the same
token within 60 seconds is the retry above. Refreshes and revocations run one at a time,
and a request waits for its turn as the request it is: a client that hangs up while
waiting gives up its place and nothing is done for it, so the work that waits is never
more than the connections that are open. Only from its turn on is a request finished
without its client.

Before this was so, a hang-up could leave two unspent tokens of one family in the file.
Such a file is settled where the rows are read at a start: the token stored last, which is
the one the client holds, stays unspent, and the other is marked replaced, so that using
it ends the family.

### Lifetime

A family ends when its usable token is 30 days old. Every exchange starts the 30 days
again, and a working client exchanges at least every 7 days, when its access token runs
out. There is no absolute lifetime, on purpose: an agent that keeps working is never asked
for a person again. A family nobody uses is gone after 30 days, the same time after which
an account's server alerts go ([ADR-0042](/chartgen/decisions/0042-server-side-alerts/)).

The 30 days are checked where a family is used: a refresh, a revocation, an access token
from before the restart on its first request, and the counts for a client's and a user's
limits each look at the families they are about and end the ones that are over. So a
family past its 30 days is refused whether or not anything has removed it yet. What nobody
asks about any more is removed by a sweep every 10 minutes, with its access token from
before the restart. The sweep is the first periodic task that takes the lock every
authenticated request needs. It holds it for one look at each family, not at their tokens,
writes nothing to the store and waits for nothing while it holds it. The rows of an
expired family leave the file at the next start, as before.

An exchange counts as getting a token: it is recorded as the client's and the user's
activity, as a code exchange is. So a working agent keeps its owner's server alerts alive,
and one that stopped lets them expire. A tool call still writes nothing.

### Revocation

- **By whoever holds a token:** `POST /revoke`
  ([RFC 7009](https://www.rfc-editor.org/rfc/rfc7009)) with `token` and `client_id`. A
  refresh token, or an access token that came from one, ends the whole family and its access
  tokens. Any other access token ends alone. The rows are deleted before the answer, so it
  holds after a restart, and `503` means nothing was revoked. The answer is `200` for a
  token the server does not know or one of another client, which is left alone, so the
  endpoint tells nobody which tokens exist.
- **By the server:** a token of the family that is no longer its usable one comes back, as
  above.
- **By an operator, for a token nobody holds any more:** delete the rows and restart. The
  families are loaded at startup, so a deleted row is a family that no longer exists. The
  [OAuth reference](/chartgen/reference/oauth/#revoking-a-refresh-token) has the statement.
  It says to stop the server first. If the rows are deleted while it runs, a refresh in
  that family is answered `503` and stores nothing, so it cannot write the family back
  before the restart.
- **With the client:** an evicted or deleted client takes its refresh tokens along. When a
  user's approval of a client is dropped, which today only the limit of 50 approvals does,
  that user's families for that client end too.

**What this gives up:** a restart used to end every access token, and operators could rely
on that, for instance after a client id leaked. For a client with a refresh token it no
longer does. The ways above are the only ones that end such an access token before its 7
days are over, so each of them ends it together with the family: `/revoke`, a spent token
that comes back, the rows deleted by an operator, the client evicted or deleted, the
approval dropped, and the family running out.

Signing out of the web app does not end a family, as it does not end a connector's access
token today ([ADR-0044](/chartgen/decisions/0044-sign-in-through-github-and-google/)): an
agent is there to work with no browser open.

There is no page in the web app that lists an account's connected applications and ends
one. Until there is, a user who lost the machine an agent ran on needs the operator.

### Limits

A family counts as a token of its client, in every limit that counts tokens.

- **With sign-in on,** per user: 20 families per client, the one longest out of use ended
  first, and a client held only through a family is one of the user's 20 live clients. The
  client that drops out of those 20 loses that user's families. The family just issued or
  just exchanged is never the one ended. Family times are whole seconds; among families
  equal to the second, the one with the smaller id goes first, which is arbitrary but the
  same every time for the same data.
- **With sign-in off,** 20 families per client, and at the limit the code exchange answers
  **without a refresh token** instead of ending the oldest. A client id is all it takes to
  authorize there, so ending the oldest would let anyone who has read a client id end the
  family of the agent that uses it.
  [ADR-0036](/chartgen/decisions/0036-own-origin-invalid-client-redirect/): "A stranger must
  not be able to end a session in use."
- **A family has at most two access tokens:** the newest, and for 60 seconds after a
  refresh the one before it. Anything older ends at the refresh. Each refresh used to add
  a 7-day token to the ones before, without limit where no user's limits apply. The one
  before is not ended at once, because a request sent with it just before the refresh can
  arrive just after, and a `401` for a token the client believes valid is what sends it
  back to a browser. Revoking, a token that comes back and every other way a family ends
  take both at once, with no 60 seconds. A restart brings back only the newest. After a
  retry, the one kept for the 60 seconds is the access token the client was using before
  the exchange, and the one that went out in the answer it never got ends.
- **Access tokens with sign-in off** had no limit either: a loop of `/authorize` and
  `/token` for one client minted 7-day tokens without end, and every request walks all of
  them. A client now keeps 100 that belong to no family, the web app's number with sign-in
  on, and one more drops the oldest by issue, since without a user a token has no time of
  last use. A family's tokens are never dropped for this, so nobody who knows a client id
  can push out the token of the agent that uses it; what they can push out are other
  family-less tokens, whose holders authorize again with no person, sign-in being off.
  With the two per family above, a client holds at most 40 plus 100.
  **What is left:** a client that could get no refresh token, because others hold its 20
  families, has only tokens under this limit, so anyone with its client id can push its
  token out by minting 100. Dropping was still chosen over refusing at the limit, because
  refusing would let the same stranger lock the web app out of its own client for good,
  where a dropped token costs one more authorization, with no person while sign-in is off.
- **Pending codes with sign-in off** had no limit: every `/authorize` for a registered
  client stored a code for 5 minutes, with a `code_challenge` of any length. Now a client
  keeps 20, its own oldest dropped first, the server keeps 2000 and then answers `429` with
  `Retry-After: 60` instead of dropping someone else's, and a `code_challenge` must be 43
  base64url characters, in both modes. A `code_challenge_method` other than `S256` is
  refused there as well: it used to be accepted and treated as S256. With sign-in off 100 free client ids can hold
  `/authorize` at `429` for everyone while they keep asking, bounded by `/register`'s rate
  limit per address, and a browser sent there meanwhile lands on that answer with no way
  back. That is left as it is here: a public server should have sign-in on.
- **A family keeps its client from being evicted,** as a live access token does. After a
  restart no access token is left in memory, and a refresh token is all a headless client
  has. Evicting its client then would cost it its authorization, and a person.

### Sign-in on, off, and switched

Refresh tokens are issued in both modes. With sign-in off, a client id already yields a
token to anyone who has it, so a refresh token protects nothing there that the client id
does not already expose. It only saves the client the code flow after a restart.

A refresh token is refused when the server's sign-in mode is not the one it was issued
under. One from before sign-in would otherwise yield a token without a user on a server
where every token has one, owned by its client: the path by which rows from before sign-in
could be reached through a client id. The rows are kept and ignored, so switching back
finds them again.

Refreshing claims nothing. The rows from before sign-in go to `CHARTGEN_CLAIM_OWNER` when
that user gets a token, and a refresh token is that user's or it is not.

### No store

In a build without the `scripting` feature, or when `CHARTGEN_DB` cannot be opened, the
grant works and the families live in memory, as sessions do. They end with the process, and
for a store that cannot be opened startup says so. Durability needs the file on a volume that outlives the container, as
registered clients already do.

### Consequences

- Good: a headless client is authorized once. A restart or a deploy costs it nothing: its
  access token still works, and it refreshes when the 7 days are over.
- Good: connectors stop sending their users through sign-in every 7 days.
- Good: nothing durable is first-party, and no new path leads to rows owned by a client.
- Good: a copied refresh token is noticed. Of two holders, the one who exchanges second
  presents a token that is no longer the usable one, and the family ends, however many
  exchanges the other has made in between.
- Bad: **one authorization can last for good.** While an agent keeps refreshing, its
  account's access and server alerts never need another sign-in, unlike a browser session,
  which ends after 30 days whatever happens. The ways out are the ones under Revocation.
- Bad: a copied refresh token that is used while its owner is idle works until the owner
  refreshes, or for 30 days after the copy stops.
- Bad: **a full client table no longer clears by restarting.** A family pins its client
  through restarts, and for 30 days out of use where an access token pinned it for 7. With
  sign-in on the price is unchanged, 20 clients per account. With sign-in off someone
  holding the table full no longer has to come back every week. The
  [recovery](/chartgen/reference/oauth/#when-register-stays-at-503) now deletes refresh
  tokens too.
- Bad: with sign-in off, a client whose 20 families are held by others gets no refresh
  token until one of them is 30 days out of use.
- Bad: a client that crashes after an exchange and before saving the answer has 60 seconds
  to retry. After that the token it still has is a spent one, presenting it ends the
  family, and a person authorizes again.
- Bad: **a restart no longer ends the access token of a client with a refresh token.**
  Ending one early takes one of the ways under Revocation.
- Bad: **what the server cannot fix.** A client must know when its access token runs out,
  to refresh in time. The Python SDK above keeps that time in memory only, so a client
  process that is restarted and then holds the same access token past its 7 days gets a
  `401` and authorizes again, with a person. A process that runs through refreshes in time,
  and so does a client that stores the time, as Hermes does. A client that answers a `401`
  with a refresh needs none of this.
- Bad, and not new: **`/token` and `/revoke` have no rate limit.** What one such request
  costs does not grow with what the server holds: a refresh token names its family, the
  families are kept by id, and a request looks at the tokens of that one family, or at none
  if it names no family the server has. A first version kept all tokens in one table and
  walked it, three times for a refresh token the server did not know, while holding the
  lock every authenticated request needs: 0.6 s at the 2 million rows that the limits
  then allowed with sign-in off. Who holds which family is kept per client and per user, for
  their limits.
- Bad, and not new: **every authenticated request walks the access tokens in memory,** to
  drop the expired ones, as it did before this decision, and so do the per-user limits
  when a token is issued. Access tokens are capped per family, per client and per user
  and are never in the file. Measured: 1.3 ms for 100,000 of them, 10 ms for 240,000.
  Left as it is here.
- Neutral: an event stream still ends with the process and reconnects, as it already had
  to.
- Neutral: the answers of `/token` and `/register` and the metadata document name the new
  grant also on a server with nothing configured. A client that ignores a
  `refresh_token` behaves as before.

### Testing

**The restart** is proved with the real binary, in `tests/e2e.rs`. With sign-in off, a
client is authorized, the process is killed and started again on the same `CHARTGEN_DB`, and
the same access token calls a tool, with no `/token` request in between. Then the refresh
token yields the next pair, twice in a row with a restart between, and an access token from
before a refresh gets `401`. The same test revokes and restarts once more, and neither token
works. With sign-in on, the second test starts the binary on a file that already holds a
registered client, a user, a session and an approval: the connector's access token is known
again as its user, the web app's is not, and the refresh token works.

**A real client** was run by hand, as described under the decision: the Python SDK against a
local server, three runs with a restart before the third. It is not part of the test suite.

**A deploy** is a new build started on the file the last one left, which is those restarts
plus a schema change. The schema part is a store test: a file from before this migration,
with clients, users and approvals in it, is opened by the current code and takes a refresh
token. It is built from the migrations before this one, found by content and not by number.
A real-binary test would have to set `PRAGMA user_version` to a fixed number, and would
break the day a later migration is appended.

Further tests cover a failed store write, a spent token after a restart, the retry, rows
deleted by an operator, the web app's client, the workspace endpoint, signing out, both
sign-in modes and the switch between them, the limits, a dropped approval, and that
refreshing claims nothing. Where one of these ends a family, the test also restarts and
checks that the access token is gone. Others cover the 7 days across a restart, and an event
stream and a chart subscription opened with an access token from before one.
