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

Only `authorization_code` with PKCE S256 is supported — there is no
`refresh_token` or `client_credentials` path.

## Token lifetime

Access tokens are valid for **7 days** (`604800` seconds). Expired tokens are
evicted lazily on the next request; there is no refresh-token flow, so the
client re-runs the authorization code dance when a token expires.

Clients and tokens are held in memory — a process restart invalidates every
registration and token, and Claude.ai will re-register transparently.

## MCP handler behavior

| Endpoint | Behavior |
|----------|----------|
| `GET /` | Health / discovery JSON. Unauthenticated. |
| `POST /mcp`, `POST /`, `POST /message` | JSON-RPC over HTTP. `tools/list` is accepted unauthenticated so the connector can enumerate tools before OAuth completes; `tools/call` requires a valid bearer token. |
| `GET /mcp`, `GET /sse` | Persistent SSE stream. Pushes `notifications/alert_triggered` JSON-RPC messages to [subscribed clients](/chartgen/guides/notifications/). |

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
