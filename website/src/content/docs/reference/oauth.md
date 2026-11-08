---
title: OAuth 2.1 PKCE
description: OAuth endpoints, client registration, and token lifetime for the Claude.ai connector flow.
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
| `GET` | `/authorize` | Authorization endpoint. Requires `client_id`, `redirect_uri`, `response_type=code`, `code_challenge`, `code_challenge_method=S256`, optional `state`. |
| `POST` | `/token` | Token endpoint. Exchanges `code` + `code_verifier` for an access token. |
| `GET` | `/oauth/signin`, `/oauth/login/{provider}`, `/oauth/callback/{provider}`, `/oauth/cancel`, `/oauth/consent` | [Sign-in](#sign-in) pages and redirects. `404` with sign-in off. |
| `POST` | `/oauth/consent`, `/oauth/logout` | The consent decision, and signing out (`204`, clears the cookie). `404` with sign-in off. |
| `GET` | `/oauth/signed-out` | The page after signing out. `404` with sign-in off. |
| `GET` | `/oauth/me` | Who a bearer token belongs to: `{"sign_in": true, "user": {"id": "github:1", "name": "…", "provider": "github"}}`, `401` with `user: null` for a bad token, and `{"sign_in": false, "user": null}` with sign-in off. |
| `POST` | `/workspace/sync` | The web app's sync of a user's workspace ([ADR-0050](/chartgen/decisions/0050-per-user-workspace-sync/)). `{"since": 0, "writes": [{"key", "value" or null, "base_version"}]}` answers `{"cursor", "restored", "results": [{"key", "status": "ok", "version"}, or "conflict" with "version" and "value", or "too_large", "quota", "bad_key"], "changes": [{"key", "value", "version"}]}`. `"restored": true` says the cursor was past this account's versions. `400` for more than 50 writes in one request, `401` for a bad token, `403` for another application's, `408` for a body not sent within 8 s, `413` past 1 MiB, `429` with `Retry-After` past 120 requests or 8 MiB a minute per user, for a minute after a request held its place by neither sending nor reading for 8 to 10 s, and while another sync of the user or too many in all are running, `503` when the store fails, `404` with sign-in off or no store. The token and these limits come before the body is read. Only the keys the app syncs are stored, others answer `bad_key`. |
| `GET` | `/favicon.svg`, `/favicon.ico` | The g of the wordmark, as SVG and as a 16, 32 and 48 px icon. The SVG is the connector icon surfaced by `logo_uri`. |

## Metadata document

```json
{
  "issuer": "https://chartgen.example.com",
  "authorization_endpoint": "https://chartgen.example.com/authorize",
  "token_endpoint": "https://chartgen.example.com/token",
  "registration_endpoint": "https://chartgen.example.com/register",
  "response_types_supported": ["code"],
  "grant_types_supported": ["authorization_code"],
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

Only `authorization_code` with PKCE S256 is supported — there is no
`refresh_token` or `client_credentials` path.

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

A code with a user works once, also after a failed attempt. Without sign-in a code stays
valid until it expires, as it always has.

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
evicted lazily on the next request; there is no refresh-token flow, so the
client re-runs the authorization code dance when a token expires.

Tokens are held in memory, so a process restart invalidates every token and
clients re-authorize. Registered clients are also stored in `CHARTGEN_DB` and
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

If every client without scripts holds a live token, nothing can be evicted. New
browsers, browsers recovering from a stale client and new MCP clients then cannot
sign in, while clients already registered keep working. The server logs this at most
once a minute:

```text
[OAuth] /register refused: client table full at 1000 clients, 12 refusal(s) since the last line. ...
```

**Usually this clears without deleting anything, so do not start by deleting
registrations.** A token expires after 7 days, and its client can then be evicted
unless it has a new one. To clear it sooner, restart the server. A restart drops every
token, so the next registrations can evict clients without scripts, and clients in use
sign in again by themselves.

**Delete only if it keeps coming back.** If `/register` returns to `503` soon after a
restart, something is keeping every client live. `/authorize` and `/token` issue new
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
   sqlite3 /host/path/to/chartgen.db "DELETE FROM oauth_clients WHERE client_id NOT IN (SELECT user_id FROM user_indicators);"
   ```

   A wrong path changes nothing: `sqlite3` creates an empty file there and fails with
   `no such table: oauth_clients`. Remove that file and use the right path.
3. Start the server.

This removes every client that owns no scripts, and clients that own scripts stay.
The removed ones recover like evicted clients, and keeping the table full then takes
filling it again through the rate-limited `/register`.

**A table full of script owners is different.** Owners are never evicted, so neither
waiting nor a restart frees anything, and the statement above deletes nothing. A slot
only comes free by choosing whose scripts go. With the server stopped, remove that
client from both tables:

```bash
sqlite3 /host/path/to/chartgen.db "DELETE FROM user_indicators WHERE user_id = '<client_id>'; DELETE FROM oauth_clients WHERE client_id = '<client_id>';"
```

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
[OAuth] sign-in started provider=github
[OAuth] sign-in succeeded provider=github
```

MCP requests are tagged `[MCP]` and note whether an `Authorization` header was
present. Logs never contain provider secrets or codes, `state` values, session values or
user ids.
