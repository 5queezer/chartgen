---
title: OAuth 2.1 PKCE
description: OAuth endpoints, client registration, token lifetime and refresh tokens, for connectors and headless agents.
---

`chartgen --serve` exposes an OAuth 2.1 authorization server with PKCE (S256).
Claude.ai drives the flow automatically — this page documents the
endpoints and behavior for anyone integrating a different client or debugging
a connector.

All URLs are built from `CHARTGEN_BASE_URL` (falls back to
`http://localhost:<port>` if unset). See the [deployment guide](/chartgen/guides/deploy/).

By default `/authorize` approves every request, so a `client_id` is all it takes to get
a token. With a provider configured it needs a [signed-in user](#sign-in) instead.

## Endpoints

| Method | Path | Purpose |
|--------|------|---------|
| `GET` | `/.well-known/oauth-protected-resource/mcp` | RFC 9728 protected-resource metadata for `/mcp`: `resource` is `<base>/mcp`, `authorization_servers` is `[<base>]`. |
| `GET` | `/.well-known/oauth-protected-resource` | The same for the deprecated root alias (`resource` is `<base>`). |
| `GET` | `/.well-known/oauth-authorization-server` | RFC 8414 metadata document. |
| `POST` | `/register` | Dynamic client registration (RFC 7591). Returns a `client_id`/`client_secret`. |
| `GET` | `/authorize` | Authorization endpoint. Requires `client_id`, `redirect_uri`, `response_type=code`, `code_challenge`, `code_challenge_method=S256`, optional `state`. `code_challenge` is the SHA-256 of the verifier in base64url without padding: exactly 43 characters of `A-Z a-z 0-9 - _`, or `400 invalid_request`. `code_challenge_method` may be left out; any value but `S256`, such as `plain`, is `400 invalid_request` too. |
| `POST` | `/token` | Token endpoint. Exchanges `code` + `code_verifier` for an access token and, for every client but the web app's own, a [refresh token](#refresh-tokens). Exchanges a refresh token for the next pair. |
| `POST` | `/revoke` | [Revocation](#revoking-a-refresh-token) (RFC 7009): `token` and `client_id`, as a form or JSON. |
| `GET` | `/oauth/signin`, `/oauth/login/{provider}`, `/oauth/callback/{provider}`, `/oauth/cancel`, `/oauth/consent` | [Sign-in](#sign-in) pages and redirects. `404` with sign-in off. |
| `POST` | `/oauth/consent`, `/oauth/logout` | The consent decision, and signing out (`204`, clears the cookie). `404` with sign-in off. |
| `GET` | `/oauth/signed-out` | The page after signing out. `404` with sign-in off. |
| `GET` | `/oauth/me` | Who a bearer token belongs to: `{"sign_in": true, "user": {"id": "github:1", "name": "…", "provider": "github"}}`, `401` with `user: null` for a bad token, and `{"sign_in": false, "user": null}` with sign-in off. |
| `POST` | `/workspace/sync` | The web app's sync of a user's workspace ([ADR-0050](/chartgen/decisions/0050-per-user-workspace-sync/)). `{"since": 0, "writes": [{"key", "value" or null, "base_version"}]}` answers `{"cursor", "restored", "results": [{"key", "status": "ok", "version"}, or "conflict" with "version" and "value", or "too_large", "quota", "bad_key"], "changes": [{"key", "value", "version"}]}`. `"restored": true` says the cursor was past this account's versions. `400` for more than 50 writes in one request, `401` for a bad token, `403` for another application's, `408` for a body not sent within 8 s, `413` past 1 MiB, `429` with `Retry-After` past 120 requests or 8 MiB a minute per user, for a minute after a request held its place by neither sending nor reading for 8 to 10 s, and while another sync of the user or too many in all are running, `503` when the store fails, `404` with sign-in off or no store. The token and these limits come before the body is read. Only the keys the app syncs are stored, others answer `bad_key`. |
| `GET` | `/favicon.svg`, `/favicon.ico` | The g of the wordmark, as SVG and as a 16, 32 and 48 px icon. The SVG is the connector icon surfaced by `logo_uri`. |

### Other origins, framing and sniffing

A page on another origin, such as an MCP client that runs in a browser, is answered on
these routes only ([ADR-0062](/chartgen/decisions/0062-cors-per-route-and-no-framing/)):
`/mcp` and its aliases `/`, `/message` and `/sse`, `/register`, `/token`, `/revoke` and
the three `/.well-known/` documents. There every answer carries
`Access-Control-Allow-Origin: *`, a preflight allows any method and header, and
credentials are never allowed: these routes take a bearer token or nothing.

Every other route, the web app's files and an unknown path send no `Access-Control-`
header, and a preflight to them is not `200`. That covers `/authorize`, everything
under `/oauth/`, and `/workspace/sync`. Until this change they carried the same headers
as the routes above; a page on another origin that asked `/oauth/me` no longer gets an
answer it may read. Pages on the server's own origin are not affected.

The web app's pages carry `X-Frame-Options: DENY` and `Content-Security-Policy:
frame-ancestors 'none'`, and the sign-in and consent pages a stricter policy with the
same two, so no other site can show them in a frame. Every response carries
`X-Content-Type-Options: nosniff`.

## Metadata document

```json
{
  "issuer": "https://chartgen.example.com",
  "authorization_endpoint": "https://chartgen.example.com/authorize",
  "token_endpoint": "https://chartgen.example.com/token",
  "registration_endpoint": "https://chartgen.example.com/register",
  "revocation_endpoint": "https://chartgen.example.com/revoke",
  "revocation_endpoint_auth_methods_supported": ["none"],
  "response_types_supported": ["code"],
  "grant_types_supported": ["authorization_code", "refresh_token"],
  "code_challenge_methods_supported": ["S256"],
  "token_endpoint_auth_methods_supported": ["none", "client_secret_post"],
  "logo_uri": "https://chartgen.example.com/favicon.svg"
}
```

A `401` from `POST /mcp` or `GET /mcp` carries
`WWW-Authenticate: Bearer resource_metadata="<base>/.well-known/oauth-protected-resource/mcp"`,
so a client given `https://<domain>/mcp` finds the protected-resource
document and from it the authorization server. A `401` from the root and
`/message`/`/sse` aliases points at the root document instead.

The grants are `authorization_code` with PKCE S256 and
[`refresh_token`](#refresh-tokens). There is no `client_credentials` path.

## Sign-in

With `CHARTGEN_GITHUB_CLIENT_ID`/`_SECRET`, `CHARTGEN_GOOGLE_CLIENT_ID`/`_SECRET` or both
set ([setup](/chartgen/guides/deploy/#sign-in-through-github-and-google)), chartgen still
issues its own codes and tokens, so clients change nothing beyond a browser login. The
reasoning, and what a stranger with an account can still do, are in
[ADR-0044](/chartgen/decisions/0044-sign-in-through-github-and-google/).

After `/authorize` has checked the client and `redirect_uri`:

- **No session:** the request is held on the server, and the browser goes to a choice of
  provider, or straight to the only one. The provider's approval opens a session and
  the flow continues as below. Discovery, `WWW-Authenticate` and `/token` are unchanged.
- **A session and an approved client:** the code comes back at once, as without sign-in.
- **A session and a new client:** a consent page names the client and its redirect host.
  Approving remembers the answer for that user and client. Denying answers
  `error=access_denied` on `redirect_uri`.
- **The web app's own client,** whose `redirect_uri` is exactly `/` on the base URL's
  origin, needs no consent when the request began on our own origin
  (`Sec-Fetch-Site: same-origin`). From anywhere else it is asked like any client.

A denied login, a provider that refuses or cannot be reached (10 s timeout) and any other
failure end on a page with **Try again** and **Cancel**, which answers
`error=access_denied`. An expired, replayed or foreign `state` gets a page saying so, and
no session. All of these depend on cookies.

| Cookie | Holds | Lifetime |
|---|---|---|
| `chartgen_session` | The session, stored on the server only as a hash | 30 days, absolute; survives restarts |
| `chartgen_browser` | What ties a sign-in in progress to this browser | 10 minutes |

Both are HttpOnly, SameSite=Lax and Path=/. With an https base URL they are also Secure and
named `__Host-chartgen_session` and `__Host-chartgen_browser`.

A code works once, also after a failed attempt, with sign-in on and off. A second exchange
answers `400 invalid_grant`, and the client authorizes again. Without sign-in a code used to
stay valid for its 5 minutes
([ADR-0058](/chartgen/decisions/0058-refresh-tokens-for-headless-clients/)).

A code not exchanged within 5 minutes expires. With sign-in on, a user keeps 20 pending
codes. With sign-in off, where every `/authorize` for a registered client issues one:

- **20 pending codes per client.** One more drops that client's oldest, never another
  client's.
- **2000 on the server.** Past that `/authorize` answers `429 temporarily_unavailable` with
  `Retry-After: 60`, and stores nothing: a code somebody is about to exchange is not
  dropped to make room.

**What a stranger can do with this, with sign-in off.** Client ids are free there, so 100
registered clients at 20 codes each hold `/authorize` at `429` for everyone, for as long as
they keep asking: a code is gone after 5 minutes. What bounds it is `/register`'s limit of
20 clients per address per 10 minutes, so it takes 5 addresses, or one address and 50
minutes. Loopback is exempt from that limit, so behind a proxy on the same host it bounds
nothing unless `CHARTGEN_TRUSTED_PROXY_HEADER` is set. Clients that hold a token are not affected. A browser that is sent to `/authorize`
meanwhile, such as the web app on a page load, **lands on the `429` answer, a page of JSON
with no way back**: the web app cannot recover from it by itself, as it does from an unknown
client. Reloading after a minute works once the codes have expired. With sign-in on, none
of this applies. A public server should have sign-in on.

A user is the provider plus its immutable id, `github:<id>` or `google:<sub>`. One person
with both accounts is two users with separate scripts; accounts are not linked.

`POST /oauth/logout` ends the session and the web app's tokens from it. Tokens of other
clients run until they expire, and [server alerts](/chartgen/guides/web/#alerts-on-the-server)
keep running.

Limits per user: 20 clients with live tokens and 20 tokens per client, 100 for the web app
(past these the longest out of use are dropped), 20 pending codes, 20 sessions, 50 approved clients, 50
[scripts](/chartgen/guides/scripted-indicators/), and 600 `tools/call` per minute (`429`
with `Retry-After`). Sign-in: 120 steps that create state per minute per client address, loopback
exempt, one provider round trip per request, 5 open requests per browser and per user and 50 per
address with the oldest dropped, and 2000 on the server (`429` page). An IPv6 /64 is one address.
The address is the TCP peer, unless
[`CHARTGEN_TRUSTED_PROXY_HEADER`](/chartgen/guides/deploy/#sign-in-through-github-and-google) names
a proxy's header, which `/register` and the chart push cap use too.

## Unknown `client_id`

`GET /authorize` with a `client_id` the server does not know answers `400`
with `{"error":"invalid_client","error_description":"Unknown client_id"}`,
without redirecting. External MCP clients with their own redirect URIs always
get this, as RFC 6749 section 4.1.2.1 requires.

There is one narrow exception, so that the [web app](/chartgen/guides/web/)
can recover from a stale client id after a restart
([ADR-0036](/chartgen/decisions/0036-own-origin-invalid-client-redirect/)). The
server answers `302` to `redirect_uri` with `error=invalid_client` and the
original `state` added to its query string when all of these hold:

- `CHARTGEN_WEB_DIR` is set, so the server serves the web app.
- `redirect_uri` parses to exactly the origin (scheme, host, port) of
  `CHARTGEN_BASE_URL`, or of the `http://localhost:<port>` fallback. The
  `Host` header plays no part.
- `redirect_uri` has no userinfo and no fragment.

## Token lifetime

Access tokens are valid for **7 days** (`604800` seconds). Expired tokens are
evicted lazily on the next request. A client with a [refresh token](#refresh-tokens)
exchanges it for a new one, and the web app runs the authorization code flow again.

A restart ends the web app's access tokens, which are held in memory only, and the
web app re-authorizes by itself. The access token of a client with a
[refresh token](#refresh-tokens) is known again after a restart, for what is left of
its 7 days. Registered clients are also stored in `CHARTGEN_DB` and
restored at startup, with or without `CHARTGEN_SCRIPTING=1`, so a client keeps
its `client_id` across a restart
([ADR-0036](/chartgen/decisions/0036-own-origin-invalid-client-redirect/)). With
scripting on, that `client_id` also owns the client's
[scripted indicators](/chartgen/guides/scripted-indicators/)
([ADR-0025](/chartgen/decisions/0025-user-indicator-registry/)), unless sign-in is on,
when the user owns them ([ADR-0044](/chartgen/decisions/0044-sign-in-through-github-and-google/)).

Registrations only survive if the file does: a production container must
[mount a volume at `~/.chartgen`](/chartgen/guides/deploy/#docker). Without one,
or when the store cannot be opened (logged, not fatal), or in a build without
the default `scripting` cargo feature, clients live in memory only and a restart
forgets them. Claude.ai then re-registers transparently, and the web app
recovers as described [above](#unknown-client_id).

`/register` refuses redirect URIs that are not absolute URIs, such as `/relative`,
`127.0.0.1:1234/cb` or `my_app://cb` (a scheme cannot contain `_`), and `javascript:`,
`data:` and `vbscript:` ones, all with `400 invalid_redirect_uri`. It allows native
schemes such as `cursor://` and `vscode://`. `/authorize` answers `400 invalid_request`
for such a URI on a client stored before these checks existed. `/register` is also
bounded: 1–5 `redirect_uris` of at most 512 characters, a
`client_name` of at most 256 (`400 invalid_client_metadata` otherwise), 20
registrations per client address per 10 minutes (`429 slow_down`; loopback
peers are exempt) and 1000 registered clients. A client that never receives a
token within 24 hours is forgotten, unless it owns scripts. Logs never contain client ids, codes or
verifiers.

At 1000 clients, `/register` makes room where it can
([ADR-0036](/chartgen/decisions/0036-own-origin-invalid-client-redirect/)). It
evicts a client that never received a token, oldest first, or else the one longest
out of use, from memory and from `CHARTGEN_DB`. A client with a live token is never
evicted, so a session in use cannot be revoked this way, and neither is a client
with a [refresh token](#refresh-tokens) used in the last 30 days, or one
that owns stored [scripted indicators](/chartgen/guides/scripted-indicators/). With
sign-in on, one user keeps at most 20 clients live, and scripts belong to users, so only
scripts from before sign-in still pin a client.
`/register` answers `503 temporarily_unavailable` when no other client is left, or
when the owners cannot be read from the store. Clients already registered keep
working.

An evicted client loses its pending codes, and its next `/authorize` gets the
[unknown client](#unknown-client_id) answer. It recovers as after a restart without
a volume: the web app re-registers by itself and an MCP client registers again.

### When `/register` stays at `503`

If every client without scripts holds a live token or a refresh token, nothing can be evicted. New
browsers, browsers recovering from a stale client and new MCP clients then cannot
sign in, while clients already registered keep working. The server logs this at most
once a minute:

```text
[OAuth] /register refused: client table full at 1000 clients, 12 refusal(s) since the last line. ...
```

**Usually this clears without deleting anything, so do not start by deleting
registrations.** A token expires after 7 days and a refresh token 30 days after its last
use, and the client can then be evicted unless it has a new one. A restart drops the
access tokens of clients without a refresh token, so the next registrations can evict
those that own no scripts, and browsers in use sign in again by themselves. Clients with a refresh token
stay through a restart ([ADR-0058](/chartgen/decisions/0058-refresh-tokens-for-headless-clients/)).

**Delete only if it keeps coming back.** If `/register` returns to `503` soon after a
restart, something is keeping every client live: refresh tokens, which survive a restart,
or new tokens. `/authorize` and `/token` issue new
tokens with no `/register` and no rate limit, so someone who filled the table can keep
it full through restarts and expiry, and it lasts until an operator acts. The server
cannot tell this from 1000 clients really in use, and deleting real clients only makes
them register again.

1. Stop the server. A running server keeps its clients in memory and would not
   notice.
2. Find the file `CHARTGEN_DB` names, by its path on the host. With the named volume in
   the [deploy guide](/chartgen/guides/deploy/#docker), `docker volume inspect
   chartgen-data` shows the directory as `Mountpoint`. The Docker image has no
   `sqlite3`, so run this from the host, with your path in place of the placeholder:

   ```bash
   sqlite3 /host/path/to/chartgen.db "DELETE FROM refresh_tokens WHERE client_id NOT IN (SELECT user_id FROM user_indicators); DELETE FROM oauth_clients WHERE client_id NOT IN (SELECT user_id FROM user_indicators);"
   ```

   A wrong path changes nothing: `sqlite3` creates an empty file there and fails with
   `no such table: oauth_clients`. Remove that file and use the right path.
3. Start the server.

This removes every client that owns no scripts, with its refresh tokens, and clients that
own scripts stay. A headless client among the removed ones must be
[connected again](#connecting-a-headless-agent) by a person. The others recover like evicted clients, and keeping the table full then takes
filling it again through the rate-limited `/register`.

**A table full of script owners is different.** Owners are never evicted, so neither
waiting nor a restart frees anything, and the statement above deletes nothing. A slot
only comes free by choosing whose scripts go. With the server stopped, remove that
client from both tables:

```bash
sqlite3 /host/path/to/chartgen.db "DELETE FROM user_indicators WHERE user_id = '<client_id>'; DELETE FROM refresh_tokens WHERE client_id = '<client_id>'; DELETE FROM oauth_clients WHERE client_id = '<client_id>';"
```

## Refresh tokens

A client that is authorized once keeps working with nobody at a browser: across a
restart, a deploy and the 7 days of an access token
([ADR-0058](/chartgen/decisions/0058-refresh-tokens-for-headless-clients/)).

The code exchange answers with a `refresh_token` next to the access token:

```json
{"access_token": "…", "token_type": "Bearer", "expires_in": 604800, "refresh_token": "…"}
```

The web app's own client gets none: any client whose `redirect_uri` is exactly `/` on
`CHARTGEN_BASE_URL`'s origin. This is the same with [sign-in](#sign-in) on and off.

To get the next access token, `POST /token` as a form or JSON:

```bash
curl -s https://chartgen.example.com/token \
  -d grant_type=refresh_token -d client_id="$CLIENT_ID" -d refresh_token="$REFRESH_TOKEN"
```

The answer has the same shape, with a **new** refresh token. Store it in place of the old
one before using the access token. A refresh token is an opaque string of about 90
characters, `A-Z a-z 0-9 - _` and one `.`; do not parse it.

- **Each refresh token works once.** Presenting one that was already exchanged, however
  long ago, ends the authorization: every refresh token and access token that came from it. That is how a
  copied token is noticed. Two processes must therefore not share one refresh token; give
  each its own authorization.
- **A lost answer can be retried for 60 seconds** with the same refresh token. The retry
  gets a new one, and the one from the lost answer stops working.
- **The access token outlives a restart of the server.** The server stores the SHA-256 of
  the newest access token issued with a refresh token and knows it again at startup, so a
  restart or a deploy causes no `401`. Its 7 days count from when it was issued, and a
  restart adds nothing. Clients commonly refresh only when they know their token has run
  out, and answer a `401` with a whole new authorization, so this is what keeps a person
  out of it.
- **A refresh leaves the access token before it 60 seconds,** and ends any older one of the
  same authorization at once. A request sent just before the refresh may arrive just
  after it, and should not fail. An authorization so never has more than two access tokens.
- **30 days out of use ends it.** Every exchange starts the 30 days again, so a client
  that keeps working is never sent back to a browser.
- It belongs to the client it was issued to, and with sign-in on to the user who
  authorized. Its access tokens have the same owner as the first one, and are never the
  web app's: `/workspace/sync` answers them `403`.
- It is not valid after sign-in is turned on or off, and works again when the setting is
  switched back.
- Signing out of the web app leaves it, as it leaves a connector's access token.
- Refreshing counts as activity for [server alerts](/chartgen/guides/web/#alerts-on-the-server),
  like signing in.

| `/token` answer | Meaning |
|---|---|
| `200` | The new pair. |
| `400 invalid_grant` | Unknown, expired, revoked, used before, another client's, or from the other sign-in setting. Authorize again. |
| `503 server_error` | The store could not be written. Nothing was issued, and the refresh token sent is still the usable one: try again. It repeats until the next start if the token's rows were [deleted from the file while the server ran](#revoking-a-refresh-token). |

**Limits.** With sign-in on, a user keeps 20 authorizations per client, the one longest
out of use ended first, within the 20 clients they keep live. The one just issued or just
refreshed is never the one ended. With sign-in off, a client
keeps 20, and past that a code exchange answers with an access token and no refresh
token: nobody's is ended to make room.

With sign-in off a client also keeps at most 100 access tokens that came with no refresh
token, which are the web app's and those of a client past its 20 authorizations. One more
ends the oldest of them by time of issue. Access tokens that came with a refresh token are
never ended for this. Together with the two per authorization, a client holds at most 140
access tokens. A client that got no refresh token, because others hold its 20
authorizations, has only tokens under this limit, so anyone who knows its client id can
push its token out by getting 100 of their own. Refusing at the limit instead would let
them lock the web app out. With sign-in on none of this applies.

`/token` and `/revoke` are not rate-limited.

**What is stored.** The server keeps the SHA-256 of a refresh token, of the part of it that
names its authorization, and of the access token issued with it, never a value, in
the [`refresh_tokens`](/chartgen/reference/persistence/) table of `CHARTGEN_DB`. The file
has to outlive the process: a container needs
[a volume at `~/.chartgen`](/chartgen/guides/deploy/#docker). Without one, when the store
cannot be opened, or in a build without the default `scripting` cargo feature, refresh
tokens and these access tokens work until the next restart and no longer. For a store that cannot be opened the
log says so at startup:

```text
[OAuth] refresh tokens stay in memory and end with this process: no store
```

### Connecting a headless agent

An agent that runs on a server, with no browser, needs a person once, for the
authorization. After that it refreshes by itself.

1. **Use `https://<domain>/mcp` as the server URL with OAuth as the auth method.** The
   client must discover the server from the `401`, register itself, run the authorization
   code flow with PKCE, **keep its tokens on disk, with the time the access token runs
   out** (`expires_in` is 604800 seconds), and refresh once that time has passed.
   With Hermes:

   ```bash
   hermes mcp add chartgen --url https://chartgen.example.com/mcp --auth oauth
   ```

   Run it as the user the agent runs as, or the token files it writes are not readable by
   the agent. In a container, for example:

   ```bash
   docker exec -it -u hermes <container> hermes mcp login chartgen
   ```

   **The first authorization needs a terminal.** Without one, `hermes mcp add --auth oauth`
   offers to continue unauthenticated and then reports `Connected`: chartgen answers
   `initialize` and `tools/list` without a token, so the connection looks fine, while every
   tool call gets `401`. If the agent lists chartgen's tools and cannot call any, this is
   what happened: run `hermes mcp login <name>` once with `-it`.
2. **Open the URL the client prints, in any browser,** also on another machine. With
   sign-in on, sign in and approve the client on the consent page. The browser is then sent
   to the client's `redirect_uri`, usually a loopback address. If the client does not run
   on the machine with the browser, the page fails to load: copy the address from the
   address bar back into the client, which is what it asks for.
3. **Check it once.** Restart chartgen and let the agent make a call. The log says
   `Refresh tokens: 1 authorizations restored` at startup, the call succeeds with no
   browser, and no `/authorize` or `/token` line follows. Seven days after the
   authorization, or earlier if the client refreshes ahead of time, the log shows
   `POST /token SUCCESS — token refreshed`.

A client that does not remember when its access token runs out, for example because it
keeps that time in memory and was restarted, sends the old token after the 7 days, gets a
`401`, and most clients then ask for a new authorization. If an agent comes back to a
browser about a week after it was connected, this is why: the fix is in the client, which
must store the expiry or refresh after a `401`.

With sign-in on, what the agent stores (scripts, server alerts) belongs to the account that
approved it, and is visible to that account in the web app. With sign-in off it belongs to
the agent's `client_id`, and anyone who learns that id can authorize as the same client, so
do not rely on a refresh token for protection there.

**Waking the agent.** An agent that has a webhook receiver can be woken by its own
[server alerts](/chartgen/reference/mcp/#waking-your-agent), with the setup an alert is about
in the body: the operator names the receiver once
([deploy guide](/chartgen/guides/deploy/#the-agents-receiver)), and the agent creates alerts
with `notify_agent: true`. The receiver needs a route whose secret is the operator's string
and which verifies Standard Webhooks signatures.

An agent that was idle for more than 30 days, or whose token files were lost, restored from
an old backup or copied to a second machine that also runs, has to be authorized again from
step 1. Keep the token files out of backups that might be restored next to a running
agent: the copy that is used second ends both.

### Revoking a refresh token

**Holding the token:** `POST /revoke` with `token` and `client_id`, as a form or JSON.

```bash
curl -s -o /dev/null -w '%{http_code}\n' https://chartgen.example.com/revoke \
  -d client_id="$CLIENT_ID" -d token="$REFRESH_TOKEN"
```

A refresh token, or an access token that came from one, ends the whole authorization at
once. Another access token ends alone. `200` also for a token the server does not know or
one that belongs to another client, which is left alone. `400 invalid_request` without
`token` or `client_id`. `503` means the store could not be written and nothing was revoked.

**Not holding it,** for example after losing the machine the agent ran on: this takes the
operator, because no page lists an account's authorizations yet.

1. Stop the server. A running server keeps refresh tokens in memory: if the rows are
   deleted under it, access tokens of that authorization go on working, and its refresh
   is answered `503` and stores nothing, both until the server is started again.
2. Delete the rows, by account or by client, with the path as
   [above](#when-register-stays-at-503):

   ```bash
   sqlite3 /host/path/to/chartgen.db "DELETE FROM refresh_tokens WHERE user_id = 'github:<id>';"
   sqlite3 /host/path/to/chartgen.db "DELETE FROM refresh_tokens WHERE client_id = '<client_id>';"
   ```

3. Start the server. An access token is only known again through its row, so nothing the
   agent held still works.

The first statement ends every application the account approved. With sign-in off there is
no account, so use the second.

## MCP handler behavior

The HTTP transport is **MCP Streamable HTTP (spec 2025-03-26)**. See the
[MCP reference](/chartgen/reference/mcp/) for the full transport description.

| Endpoint | Behavior |
|----------|----------|
| `GET /` | Health / discovery JSON. Unauthenticated. With `CHARTGEN_WEB_DIR` set, a request whose `Accept` contains `text/html` gets the [web app](/chartgen/guides/web/) instead (also the OAuth callback `/?code=…`). |
| `POST /mcp` (deprecated aliases: `POST /`, `POST /message`) | JSON-RPC 2.0 over HTTP. `tools/list` and `initialize` are accepted unauthenticated so the connector can enumerate tools before OAuth completes; every other method requires a valid bearer token. Response `Content-Type: application/json`. Clients that send `Accept` without `application/json` receive `406 Not Acceptable`. An optional `Mcp-Session-Id` header is echoed back unchanged; `initialize` without one gets a fresh id. chartgen keeps no session state beyond scoping [chart push](/chartgen/reference/mcp/#subscribe_chart--unsubscribe_chart) subscriptions to the session's SSE stream. |
| `GET /mcp` (alias: `GET /sse`) | Persistent SSE stream for server-initiated messages. Pushes `notifications/alert_triggered` JSON-RPC notification frames to [subscribed clients](/chartgen/guides/notifications/) and `notifications/chart_tick` frames for [chart push](/chartgen/reference/mcp/#subscribe_chart--unsubscribe_chart). No legacy `event: endpoint` frame is emitted — clients POST to `/mcp` directly. |

## Debugging

Server logs every OAuth step to stderr with an `[OAuth]` prefix:

```
[OAuth] GET /.well-known/oauth-authorization-server → issuer=https://...
[OAuth] POST /register client_name=Some("Claude") redirect_uris=[...]
[OAuth] GET /authorize client_id=... redirect_uri=...
[OAuth] POST /token PKCE verify: ok=true verifier_len=64 challenge=...
[OAuth] POST /token SUCCESS — token issued (len=...)
[OAuth] POST /token SUCCESS — token refreshed
[OAuth] sign-in started provider=github
[OAuth] sign-in succeeded provider=github
```

MCP requests are tagged `[MCP]` and note whether an `Authorization` header was
present. Logs never contain provider secrets or codes, `state` values, session values,
refresh tokens or user ids.
