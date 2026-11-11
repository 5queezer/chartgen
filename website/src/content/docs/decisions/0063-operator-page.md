---
title: "0063 — A read-only operator page, for a signed-in operator in the web app only"
description: "CHARTGEN_ADMIN_USERS names the user ids that may open /admin. One read, answered by a layer in front of the router to an operator's own web-app token; everyone else gets what an unknown path gets. Counts, times, ids and names only."
sidebar:
  order: 63
---

**Status:** Accepted
**Date:** 2026-11-11
**Amends:** [ADR-0044](/chartgen/decisions/0044-sign-in-through-github-and-google/) (the operator of a server can see who signed in and which clients registered; other users' display names are shown to the operator) and [ADR-0050](/chartgen/decisions/0050-per-user-workspace-sync/) (a second thing only the web app's own token may do, and one place in the code that decides which token that is)

## Context and Problem Statement

With sign-in on ([ADR-0044](/chartgen/decisions/0044-sign-in-through-github-and-google/))
anyone with a GitHub or Google account may sign in. The operator of a server could
learn who did, which clients registered and how full the tables are only by opening
the database on the host.

The operator wants a page for it. Whatever is built touches the authorization model:
it shows data about other users, to one person, on a server whose every other answer
is either for everybody or for the caller about themselves.

Who is the operator, how is the page reached, and what may it show?

## Decision Drivers

- Only the operator sees it. Nobody else learns from an answer that the server has
  such a page, or who may see it.
- An agent, a connector or any other application never sees it, the operator's own
  included: what is given to an application is the tools, not this.
- No e-mail address is stored or configured. The server stores none today.
- Nothing secret and no content of anyone's documents is shown, and a later column
  cannot leak by default.
- Reading it costs a bounded amount whatever the store holds.
- It only reads. Actions come later, and what is built now must not make them unsafe.

## Considered Options

For who the operator is:

1. A list of user ids in the environment, `CHARTGEN_ADMIN_USERS`.
2. The first user who signs in.
3. A role stored in the database.

For how it is reached:

A. A layer in front of the router that answers one path to an operator and lets
   every other request through untouched.
B. A normal route that answers "not found" itself to everyone else.
C. MCP tools.
D. A second port.

## Decision Outcome

Chosen: **1 and A**.

### Who the operator is

`CHARTGEN_ADMIN_USERS` holds user ids separated by commas (`github:<id>`,
`google:<sub>`), read at start. A request is an operator's when all of this holds:

1. sign-in is on and the server is not a replay server;
2. the request says it comes from the server's own page (`Sec-Fetch-Site:
   same-origin`);
3. its bearer token is valid;
4. the token is the web app's own;
5. the token's user is on the list, compared exactly as written.

**"The web app's own" is one place in the code.** A token's user carries a browser
session only when `TokenUser::for_code` gave it one, and that happens only for a code
that goes to exactly `/` on the server's own origin (`is_first_party`). The field is
private to the sign-in module. `/workspace/sync` rests on the same fact
([ADR-0050](/chartgen/decisions/0050-per-user-workspace-sync/)). Nothing may add a
second way to set it, and `is_first_party` is never widened: `/admin` is not a
first-party redirect.

**It is the address the code goes to that decides, not which client asked.** Any
client may register `/` on the server's origin as its redirect, and a code for it then
gets a session, with no consent step and no refresh token. That is safe because such a
code is only ever delivered to the server's own page at `/`, and the app there throws
away a `?code=` it did not ask for and never exchanges it (the browser test "a stray
code in the address is thrown away, not exchanged" in `signin-workspace.spec.ts`). To
get an operator's web-app token, someone else would have to read a code out of the
address of the operator's own tab on this origin, which is as much as controlling that
page.

So an application's token never passes, for three reasons that each hold alone:

- its code went to another address, so it has no session;
- a token made by a refresh has no session
  ([ADR-0058](/chartgen/decisions/0058-refresh-tokens-for-headless-clients/));
- the web app's own client gets no refresh token, so nothing durable is first-party.

And a fourth case: the web app's token is not kept across a restart, so a restart or
a deploy ends the page until the operator's tab signs in again, as signing out does
at once.

A list that the server cannot read as written stops the start: an entry that is not
`github:` or `google:` followed by digits, such as `*`, a login name, an e-mail
address or an empty entry. So does a list on a server without sign-in, where it could
never take effect. The message names the entry by its position and does not repeat
it. An id nobody has signed in with is not an error: it matches nobody yet.

The operator reads their own id in the web app's account menu, which now shows it
with a copy button. It is the provider's public id, the same value
`CHARTGEN_SERVER_ALERT_CLIENTS`, `CHARTGEN_CLAIM_OWNER` and
`CHARTGEN_AGENT_WEBHOOK_OWNER` take, and it is shown to its owner only. The other
ways to read it (the provider's profile, `/oauth/me`, the store) stay as documented.

### How it is reached

One read, `GET /admin/overview`. A layer in front of the whole router looks at the
method and the path first. For any other request it does nothing and takes no lock.
For this one it asks whether the request is an operator's. If so it answers; if not,
it passes the request on unchanged. No route of that name exists, so the request is
then answered as any unknown path is, by the same code: the web app's fallback where
the app is served, `404` otherwise. Status, headers and body are equal by
construction, for a missing token, a bad one, another user, an application, sign-in
off and a replay server alike. There is no `401` and no `403` on this path.

The path alone decides, compared exactly: `/admin/overview?x=1` is the endpoint;
`/admin/overview/`, another case or an encoded letter is not.

The operator's answer is JSON, marked `Cache-Control: no-store` and `Vary:
Authorization`, with `X-Frame-Options: DENY` and no CORS header
([ADR-0062](/chartgen/decisions/0062-cors-per-route-and-no-framing/)).

The answer is built with no lock of the OAuth store held and off the async workers,
because the store has one connection that token rotations also wait on. It is built
at most once in 10 seconds and by one caller at a time, from a fixed number of
statements: none per row of a table that is not found by an index.

### The page

`/admin` in the web app. The server already answers a browser's request for any
path with the app, so the address tells nothing. A tab opened there keeps a flag for
the tab (never an address: nothing stored is used as a place to go), signs in as
usual, which returns to `/`, and asks for the overview with `Accept:
application/json`. If the server answers it, the tab shows the page. If not, the tab
is the app at `/`, with no message. The app has no link to `/admin`.

The repository is public, so anyone can read that the app has such a view. What
stays unknown from outside is whether a given server has an operator, and who.

### What it shows

| | From |
|---|---|
| Server: version, start time, schema version, the store's file size, which features are on, watched pairs against their limit, chart push subscriptions and feeds, setups in all, alert events kept | the build, memory, the store |
| Users: how many; for the 200 who signed in last: id, display name, provider, created, last sign-in, last token issued, workspace documents and their size, setups open and closed, server alerts, authorisations (refresh token families), push devices | `users`, and counts over the other tables by owner; open setups from memory |
| Clients: how many of the 1000 allowed; for the 50 registered last: the first 8 characters of the id, name, registered, last token issued, whether it keeps an authorisation, whether it owns setups | `oauth_clients`, and whether rows exist elsewhere |

All of it was in the store or in memory already. No table, column or migration is
added.

"Last token issued" is when a token was last given, not when one was last used.

The table of users has no limit, so the page shows a bounded number and the total.

**Display names are personal data of other users, shown to the operator.** They come
from each user's provider profile and were stored already; this is the first place
they are shown to someone else. Whoever runs a server with an operator should say so
to its users.

### What it never shows

Tokens and refresh tokens, any hash, provider secrets and keys, the content of a
setup (thesis, drawings, levels), workspace keys and bodies, webhook URLs and push
endpoints, redirect URIs, family ids, addresses.

A client id is shown by its first 8 characters only. With sign-in off a client id is
the owner of data, and although it is not a secret that alone opens anything, nothing
on a page that only reads needs the whole of it.

The answer is assembled field by field into a fixed shape from a summary the store
returns for this purpose. No stored record or row is passed on as a whole, so a
column added later is not in the answer unless someone puts it there.

Names are text other people chose: a client's at open registration, a user's at
their provider. The server removes control characters and characters that change
the direction of text, and cuts a name to 80 characters; the page writes names as
text.

### Later: actions

None is built. When one is (revoking an authorisation, deleting a client):

- it is a `POST` under `/admin/`, answered by the same layer after the same check, so
  it cannot be added beside it;
- the check needs the bearer token, which the app holds in memory and a browser never
  attaches by itself, and `Sec-Fetch-Site: same-origin`. The session cookie must
  never become enough for anything under `/admin`;
- the layer writes one line (who, what, to what, with what result) before it answers.

### Consequences

- Good: the operator sees who signed in without opening the database.
- Good: nothing else about the server changes for anyone who is not the operator, on
  the wire or in the app.
- Good: what only the web app may do now rests on one constructor instead of on
  every place that builds a token.
- Bad: other users' display names and ids are shown to the operator.
- Bad: the operator has to learn their id and set it, and changing the list takes a
  restart.
- Bad: the page is signed out by every restart and deploy.
- Bad: 200 users and 50 clients are shown; an operator of a larger server sees totals
  for the rest and has no search.
- Neutral: the app's bundle contains the page for everyone.

### Confirmation

End-to-end tests against the running server: the operator's answer, its headers, and
that no token, whole client id or cookie value is in it; for eight kinds of other
caller, three `Accept` values and `GET`, `HEAD` and `POST`, the whole answer equal to
that of an unknown path, with the web app and without it; for the operator, every
path but the one and every method but `GET`; the page ended by signing out, by a
restart and by a changed list; three bad lists stopping the start; no page without a
list or without sign-in. Unit tests for the list, the shown names and the store's
summary with planted secrets. Browser tests with the fake providers: the operator
sees the page; another user, an application's token of the operator and a signed-out
visitor do not.

Not validated here: real providers, a store of real size, what a proxy in front does
with the answer's headers, and equality of timing beyond "the same code path".

## Pros and Cons of the Options

### 1. A list in the environment

- Good: no state, no first-run race, and the operator of the host is who decides.
- Bad: a restart to change it.

### 2. The first user who signs in

- Bad: whoever is first on a public server becomes operator.

### 3. A role in the database

- Good: changeable while running.
- Bad: needs a way to grant it, which is the action this decision does not build.

### A. A layer that lets everything else through

- Good: "equal to an unknown path" is true by construction.
- Bad: the endpoint is not in the list of routes; a reader has to know the layer.

### B. A route that answers "not found" itself

- Bad: two code paths that must be kept identical by hand, with and without the web
  app, for every method and `Accept`.

### C. MCP tools

- Bad: tools are what applications get.

### D. A second port

- Bad: a new thing to deploy and to keep closed.
