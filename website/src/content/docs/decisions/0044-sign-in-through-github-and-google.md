---
title: "0044 — Sign-in through GitHub and Google"
description: "With a provider configured, /authorize needs a signed-in user instead of auto-approving: chartgen stays the OAuth server for its clients and is itself a client of GitHub and Google. Anyone with an account may sign in. Scripts and limits move from the client to the user. Nothing configured changes nothing."
sidebar:
  order: 44
---

**Status:** Accepted; amended by [ADR-0050](/chartgen/decisions/0050-per-user-workspace-sync/) (browser data now follows the account)
**Date:** 2026-11-06
**Amends:** [ADR-0025](/chartgen/decisions/0025-user-indicator-registry/) (with sign-in on, scripts belong to the user, not the client), [ADR-0036](/chartgen/decisions/0036-own-origin-invalid-client-redirect/) (a client's liveness is limited per user, which makes the lockout cost accounts), [ADR-0021](/chartgen/decisions/0021-realtime-chart-push/) (with sign-in on, the per-client push cap counts per user, and an operator can name a proxy's address header) [ADR-0042](/chartgen/decisions/0042-server-side-alerts/) (with sign-in on, server alerts and their allowlist belong to accounts) and [ADR-0045](/chartgen/decisions/0045-web-push-for-server-alerts/) (with sign-in on, push devices belong to the user, the last outcome is kept per device, and an endpoint has one owner)

## Context and Problem Statement

chartgen is an OAuth 2.1 server with open dynamic registration, for MCP clients such
as Claude and for its own web app. `/authorize` approved everything, so anyone who
knew a `client_id` got a token, read its code straight off the `Location` header, and
acted as that client. A `client_id` was a credential that was never meant to be one,
and it owned scripts. ADR-0036 ends with this: "Requiring approval or authentication
at registration is the real alternative, and a separate project." This is that
project.

## Decision Drivers

- MCP clients and the web app keep the standard flow. They change nothing beyond a
  browser login.
- A `client_id` alone must not yield a token.
- **Nothing configured stays exactly as it is**, so local use, stdio, tests and a
  running production keep working until an operator opts in.
- No login CSRF, no open redirect, nothing that a request can switch on.
- Anyone with an account may sign in. That was decided up front, so limits have to
  hold against a stranger with a free account.

## Considered Options

**chartgen as OAuth server and as a provider's client (chosen).** It keeps issuing its
own codes and tokens, and adds a login in the middle of `/authorize`.

**Send clients to GitHub or Google directly,** naming them in the
`authorization_servers` metadata. Neither offers dynamic registration, so MCP clients
could not register, and chartgen would have to validate foreign tokens per call.

**An allowlist of users.** Rejected by decision: anyone with an account may sign in. An
optional deny or allow list stays a possible follow-up.

**Authenticate at registration.** MCP clients register unattended, with no browser.

## Decision Outcome

Chosen: **chartgen as OAuth server and as a provider's client**, because it is the only
option under which existing clients need no change.

### The flow

With at least one of `CHARTGEN_GITHUB_CLIENT_ID`/`_SECRET` and
`CHARTGEN_GOOGLE_CLIENT_ID`/`_SECRET` set, `GET /authorize`, after its checks on the
client and `redirect_uri`, no longer issues a code by itself:

1. **No session:** the request is kept on the server under a random id, and the
   browser goes to the chooser, `/oauth/signin`. With one provider there is no chooser.
2. `/oauth/login/{provider}` redirects to the provider with `state` and PKCE S256.
3. `/oauth/callback/{provider}` uses up the `state`, exchanges the code server-side
   and reads the user over TLS.
4. The request is **checked against the client again**. A client evicted meanwhile gets
   the unknown-client answer of ADR-0036. Then the code goes out, or the consent page
   comes first.

A provider with an id and no secret, or the reverse, stops the server at startup. A
typo must not leave it open. With nothing configured and a public
`CHARTGEN_BASE_URL`, startup logs a warning.

### Upstream security

- **Login CSRF.** `state` is single-use, and bound to a random `chartgen_browser`
  cookie through its hash. An attacker's finished callback opened in a victim's browser
  is refused, and no session results. On https the cookies are named `__Host-…`,
  because a sibling subdomain could otherwise plant the binding cookie.
- **No open redirect.** Only a random request id travels through the browser. Where
  the code goes is held on the server, is one of the client's registered URIs, and is
  checked again on return. The request is bound to the browser as well.
- **Failures** of any kind (denied, refused, unreachable, a 10 s timeout) end on a page
  of ours with **Try again** and **Cancel**. Cancel sends `error=access_denied` to the
  client. The value of the provider's `error` selects a message and is never shown.
- **Minimal scopes.** GitHub with no scope, which covers the public profile. Google with
  `openid profile email`.
- **Userinfo over TLS,** not `id_token` validation: the answer comes straight from the
  provider on the back channel, and this needs no JOSE dependency or key handling.
- **Provider tokens** are used for that one call, then dropped.

### Identity

The key is the provider plus its immutable id: `github:<numeric id>` or
`google:<sub>`, never a login or an email. A renamed account is the same user. Names
are display only, and Google's email is used as a name only if `email_verified`.

There is **no account linking**. One person with both accounts is two users with
separate scripts.

The id goes straight into the existing `user_indicators.user_id` and
`server_alerts.owner`, so those tables need no change, and it cannot collide with a client
id, which is a UUID. A new migration, the fifth, adds `users`, `sessions`, `client_grants`
and `oauth_clients.client_name`. It follows those of server alerts and push, because that
order is the only one that ever shipped.

### Sessions

An opaque random 256-bit id in a cookie, stored only as its SHA-256. HttpOnly,
SameSite=Lax, Secure on https, a 30-day absolute lifetime with no sliding, and it
survives restarts. Tokens stay in memory, as before, so after a restart a client
re-authorizes, and a browser does so without a provider round trip.

`POST /oauth/logout` deletes the session and clears the cookie. It also ends the
**web app's tokens from that session**, and only those. A connector approved on purpose
keeps its token until it expires, because signing out of the web app should not
disconnect Claude. There is no "sign out everywhere" yet.

The app then goes to `/oauth/signed-out`, not to itself. Reloading the app would sign in
again at once: with one provider there is no chooser, and the provider remembers the
user. For the same reason the other tabs are told through a `localStorage` event and
leave too, and they are told before the request, because the request ends their tokens.
If it fails they have left without cause, and have a way back.

The user is shown in a menu at the right end of the header. On the compact shell
([ADR-0039](/chartgen/decisions/0039-compact-layout/)) the same entries go in the
existing **More tools** menu: an avatar there cost the symbol its room, down to `BTC…`
on a 360 px phone.

### Consent

A signed-in user is asked before a client that is new to them gets a code. The page
shows the redirect host prominently and says plainly that the name proves nothing,
because the client chose it. Everything is escaped, there is no script, and framing is
refused. Its form carries a token bound to the session it was shown to, and a request
from another origin is refused.

Approval is remembered per user and client, so a connector does not ask again every 7
days when its token runs out. Redirect URIs of a client cannot change after
registration, so the approval stays meaningful.

**The web app's own client skips consent,** when the browser says the request began on our
own origin (`Sec-Fetch-Site: same-origin`), as the app's always does. That is any client
whose `redirect_uri` is exactly `/` on `CHARTGEN_BASE_URL`'s origin, whoever registered it, so
a stranger can register a look-alike. Asked from anywhere else it gets the consent page: from
another site's page, from a link in a mail or a chat (`none`, even through a redirect), or by a
browser that does not say. The header is read at `/authorize` and kept with the request,
because the provider's callback always looks cross-site. A code that reached our page anyway
is discarded there unless the page issued its `state`, and the page sends no referrer to other
sites, so the code in its address stays there.

### Existing rows

**Rows from before sign-in go to one named account, and to nobody else.**
`CHARTGEN_CLAIM_OWNER=github:<id>` or `google:<sub>` names it. When that user gets a token,
through any client, they collect every script, server alert and push device still owned by a
client, with waiting deliveries, whatever their limits, because these are theirs already. An alert's webhook
and its signing secret move with it. A client here means an owner shaped like the ids `/register`
hands out, tested positively, so another kind of owner is never swept in.

A script name they already have stays with its client, as does the second of two clients' scripts
with one name, so nothing is overwritten. Startup counts name collisions among the scripts. A
device that would be a second row for a subscription they already hold is removed, being the same
browser; under the same endpoint with other keys it stays, since that is not provably so. To move one, the
owner renames or deletes their own script and signs in again, one at a time per name.

**Unset, nothing moves.** The rows stay with their clients, where nobody can reach them, because
with sign-in on a token's owner is its user and never its client. Startup says how many rows
wait and what to set, as a count. Unclaimed scripts keep until claimed. Unclaimed alerts keep
running under ADR-0042's rules, so they expire 30 days after their client last got a token.
Those rows pin their client against eviction.

Rejected: **the first user through a client takes its rows.** This was the first version. A
`client_id` travels in `/authorize` URLs, browser history and proxy logs, and a connector's may be
shared, so a leaked id became a one-time transfer of the owner's scripts and alerts to whoever
moved first, with webhook secrets that sign requests to the owner's receiver. **Leaving the rows
for SQL** works, and is what setting nothing amounts to, but the variable saves the operator doing
it by hand.

Turning sign-in off again leaves users' scripts and alerts in the file, but out of reach,
because owners are then client ids again. Clients, sessions and users stay and work again
when it is turned back on. A user's alerts keep running meanwhile, and are deleted once 30
days pass with no sign-in or token of theirs.

### Server alerts

With sign-in on, an alert ([ADR-0042](/chartgen/decisions/0042-server-side-alerts/)) belongs
to the user: they see it from every client, and its limits add up per user, so more clients
buy no more alerts or watched pairs.

- **`CHARTGEN_SERVER_ALERT_CLIENTS` names accounts,** `github:<id>` or `google:<sub>`. A
  client id on it matches nobody, because a client owns nothing now, and startup logs how
  many entries look like that. An account on the list cannot be taken over by reading a URL,
  so this closes what ADR-0042 calls a listed client id being a credential.
- **Staying alive.** An alert is deleted after 30 days with no activity by its owner. A
  user's activity is their last sign-in or token, kept on the user, since the client table
  never matches them. A listed account's alerts are kept regardless, as before.
- **Logging out leaves alerts running.** An alert is there to work with no browser open, so
  signing out ends tokens and nothing else. It keeps firing and delivering.
- **Push devices belong to the user** too
  ([ADR-0045](/chartgen/decisions/0045-web-push-for-server-alerts/)), so a phone gets the alert
  made on the desktop, and the limits of 5 devices and 30 pushes a minute are per user. They live
  on the same activity as alerts.
- **An endpoint has one owner, whoever subscribed it last,** so a device never shows two people's
  alerts. An account subscribing an endpoint another holds removes that row. The earlier owner just
  loses that device: their alerts keep running and reach their other devices, and signing back in
  there takes it back. The app re-subscribes on load, so on a shared browser the device goes with
  whoever signs in and has notifications on. Signing out alone still leaves it, so alerts keep
  working.

  Taking one over needs the endpoint **and its keys**, which only that browser has. With either key
  different the request is refused and the existing row stays, so nobody can evict a device by
  knowing its endpoint. The same applies with sign-in off, between clients. This replaces ADR-0045's
  "two owners is two devices".
- **The last outcome is kept per device.** With several devices one shared outcome is wrong: a
  live device's 201 followed by a dead one's 410 made the tab report twice, and this device's 500
  followed by another's 201 left it silent. Each fire now records the devices it goes to, pending
  until each answers, and `subscribe_push` tells the tab its device's id. So a tab goes by its own
  outcome, waits while that is pending, and knows at once if the fire was not sent to it.

### Limits, per user

| What | Limit | Past it |
|---|---|---|
| Scripts | 50 | refused, as before; more clients no longer buy more |
| Clients holding a live token | 20 | the tokens of the client that got one longest ago are dropped |
| Tokens per client | 20, or 100 for the web app | the longest out of use dropped |
| Pending codes | 20 | oldest dropped |
| Browser sessions | 20 | oldest dropped |
| Approved clients | 50 | oldest forgotten |
| Tool calls | 600 per minute | `429` with `Retry-After` |
| Chart subscriptions | 32 | refused, as before; now per user instead of per address |

The 20 client limit is what changes ADR-0036's lockout. A client with a live token is
still never evicted, and liveness now comes from a user's token, so holding the table
of 1000 clients full takes at least 50 accounts.

The web app gets a token per page load, and each tab keeps its own, because the chart push
limits are per token, so sharing one would cost a grid its push. Its cap is roomy because a cap
below the number of open tabs would have each tab's sign-in end another's, in a loop that never
settles.

Sign-in itself is limited so that nobody can hold it shut for free:

- **Only what creates state counts:** 120 hits per minute per client address (loopback exempt,
  as for `/register`). That means an `/authorize` that stores a request, a first `/oauth/login`
  for one, and a callback with a `state` of ours. Junk costs nothing.
- **One provider round trip per request,** so round trips cannot outnumber requests.
- **5 open requests per browser and per signed-in user, and 50 per address,** the oldest of
  each going first. A client that sends no cookie is a new browser every time, so the cap per
  address is what holds. An address can only push out its own requests, and a stranger's stay.
- **2000 on the whole server, then `429`.** That takes 40 addresses to reach.
- **An IPv6 /64 is one address,** for these limits, `/register`'s and the chart push cap, because
  that is what one household or server has.
- **`CHARTGEN_TRUSTED_PROXY_HEADER`** names the header holding the client's address, such as
  `CF-Connecting-IP`, for those same limits. Unset, no header is trusted, so behind a proxy all
  clients are one address. Set, it is only as true as the proxy makes it, so the proxy must
  replace that header rather than append to it, and the server must not be reachable any other
  way. With the header absent, the address is the peer. Present but not exactly one usable
  address, it counts under one fixed bucket, so it can never reach a same-host proxy's loopback
  exemption.

### What a signed-in stranger can still do

Anyone with a free GitHub or Google account can:

- **Read everything public to a user:** charts, indicators, quotes and search.
- **Spend shared resources within the limits.** 600 tool calls a minute, which leaves
  room for four visible windows of a polling 4-chart grid
  ([ADR-0043](/chartgen/decisions/0043-multi-chart-grid/)), is real
  rendering work and upstream quota, and enough of them can put Binance or Yahoo into
  cooldown for everyone. Many accounts multiply it. There is no global call limit.
- **Take all 64 chart subscriptions** with 2 accounts, which leaves nobody push until
  they let go. The app falls back to polling.
- **Fill the store slowly.** A user row per account, and with scripting on, 50 scripts
  per account.
- **Hold the client table full** with 50 accounts: ADR-0036's lockout, at a higher price.
- **Where `CHARTGEN_SERVER_ALERTS=1`:** with the allowlist unset, hold 20 alerts per account,
  so 10 accounts use up the 200 and new ones are refused until they expire. Set to `*`, also
  make the server send webhooks, and each one shows the receiver this server's address.
  **Keep alerts off on a public server, or list the accounts you trust.**
- **Use the order tools on a `--trade` server.** `place_order` and the other engine
  tools are not per-user: everyone shares one order book and one set of engine alerts, and
  can read and cancel each other's. Orders are only recorded locally today, and nothing
  reaches an exchange. Startup logs a warning. **Do not expose a `--trade` server with
  sign-in on.** The tools need their own authorization first, certainly before they ever
  send anything.
- **Run scripts where `CHARTGEN_SCRIPTING=1`.** Sign-in supplies the login half of what
  [ADR-0024](/chartgen/decisions/0024-starlark-script-runtime/#required-before-public-hosting-a-worker-subprocess)
  requires before public hosting, not the worker-subprocess sandbox, which is still
  missing. **Keep scripting off on a public server.**

Someone without an account can still:

- **Register clients,** rate-limited per address, and with them evict clients that hold
  no live token. Registration has to stay open for MCP clients. An evicted client
  registers again. It loses its approvals, so its users see consent once more.
- **Slow new sign-ins for everyone behind one address, where no proxy header is named.**
  Registering a client and asking `/authorize` 120 times a minute uses that address's budget,
  and everybody behind the proxy shares it, as they share its 50 open requests. A same-host proxy
  has no budget, being loopback, so a flood there pushes out everyone's open requests while it
  lasts. **Set `CHARTGEN_TRUSTED_PROXY_HEADER` on a public server.**
- **Block new sign-ins for up to 10 minutes** by holding 2000 open requests, which takes 40
  IPv4 addresses or IPv6 /64s, so a botnet rather than a laptop. Refusing at that point was
  preferred to evicting strangers' requests, which would break those in progress. Anyone holding
  a session is not affected.

### Consequences

- Good: a `client_id` is no longer a credential. Knowing one yields nothing.
- Good: scripts and limits follow a person across clients and browsers.
- Good: ADR-0036's recovery still works, signed in and with no provider round trip.
- Good: the push cap stops being one pool behind a reverse proxy.
- Bad: first-time users of an MCP client see two new pages, the provider's and consent.
- Bad: chartgen now holds provider client secrets, and user ids and names in its store.
- Bad: another way to fail at startup, by design: half a provider stops the server.
- Bad: someone who already knows a browser's push endpoint, which is long and random and known
  only to that browser, its push service and this server, could register it first with made-up
  keys, and that browser's own subscribing would then be refused until their row expires. Their
  pushes would not decrypt there. A browser that gets new keys for the same endpoint under a
  different account is refused the same way; browsers change the endpoint along with the keys.
- Bad: **only what the server stores belongs to the user.** Watchlists, layouts, drawings,
  chart settings and browser-side alerts stay in the browser's `localStorage`, as before: not
  per account, not synced, and still there after signing out. The next person to sign in on
  that browser sees them. Syncing a workspace per user is a separate project.
- Bad: behind a proxy with no header named, all peers share one address, so 120 hits a minute
  comes to about 40 sign-ins a minute for the whole server.
- Bad: a browser that sends no `Sec-Fetch-Site`, such as Safari before 16.4, sees a consent page
  for the web app once per client, as does any browser on plain http off localhost.
- Bad: a tab restored from the back-forward cache misses the sign-out event, and with
  one provider and a live provider session can sign back in on its next 401.
- Neutral: an SSE stream opened before a sign-out runs on until it reconnects, as one
  does past token expiry, since tokens are checked at connect. It carries market data
  only.
- Neutral: without a store (a store that cannot be opened, or a build without the
  `scripting` feature), sign-in is still required, and users and sessions live in memory.

### Testing

Tests use a fake GitHub and Google on loopback, through
`CHARTGEN_SIGNIN_TEST_PROVIDER_ORIGIN`. Like the webhook test origin of the alerts work,
it is read from the environment only, so no request can turn it on, it is honored only
with `CHARTGEN_DATA_SOURCE=sample` and only for a loopback address, and a server on live
data ignores it.
