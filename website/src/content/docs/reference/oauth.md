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

## Endpoints

| Method | Path | Purpose |
|--------|------|---------|
| `GET` | `/.well-known/oauth-protected-resource/mcp` | RFC 9728 protected-resource metadata for `/mcp`: `resource` is `<base>/mcp`, `authorization_servers` is `[<base>]`. |
| `GET` | `/.well-known/oauth-protected-resource` | The same for the deprecated root alias (`resource` is `<base>`). |
| `GET` | `/.well-known/oauth-authorization-server` | RFC 8414 metadata document. |
| `POST` | `/register` | Dynamic client registration (RFC 7591). Returns a `client_id`/`client_secret`. |
| `GET` | `/authorize` | Authorization endpoint. Requires `client_id`, `redirect_uri`, `response_type=code`, `code_challenge`, `code_challenge_method=S256`, optional `state`. |
| `POST` | `/token` | Token endpoint. Exchanges `code` + `code_verifier` for an access token. |
| `GET` | `/favicon.svg` | Connector icon surfaced by `logo_uri`. |

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

Clients and tokens are held in memory — a process restart invalidates every
registration and token, and Claude.ai will re-register transparently.
With `CHARTGEN_SCRIPTING=1` registered clients are also stored in
`CHARTGEN_DB` and restored at startup, because the `client_id` owns that
client's [scripted indicators](/chartgen/guides/scripted-indicators/); tokens
are still lost on restart ([ADR-0025](/chartgen/decisions/0025-user-indicator-registry/)).

`/register` is bounded: 1–5 `redirect_uris` of at most 512 characters, a
`client_name` of at most 256 (`400 invalid_client_metadata` otherwise), 20
registrations per peer address per 10 minutes (`429 slow_down`; loopback
peers are exempt) and 1000 registered clients (`503
temporarily_unavailable`). A client that never receives a token within 24
hours is forgotten. Logs never contain client ids, codes or verifiers.

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
```

MCP requests are tagged `[MCP]` and note whether an `Authorization` header was
present.
