---
title: "0002 — MCP transport: Streamable HTTP 2025-03-26"
description: "Move chartgen's MCP transport from the bespoke 2024-11-05 variant to the Streamable HTTP 2025-03-26 spec."
sidebar:
  order: 2
---

**Status:** Accepted
**Date:** 2026-04-19

## Context and Problem Statement

chartgen's MCP server currently advertises `protocolVersion: "2024-11-05"`
in its `initialize` response — the HTTP+SSE transport era of the MCP spec.
The server-side implementation is, however, a hybrid that matches no
standard client exactly. `POST /mcp` returns the JSON-RPC response
synchronously in the HTTP body, while `GET /sse` streams only
server-initiated `notifications/alert_triggered` push events. The
2024-11-05 spec expects tool-call responses to come back over the SSE
channel, not in the POST body. What we built is a bespoke variant: Claude.ai
tolerates it because it is flexible about transports, but the official
`@modelcontextprotocol/sdk` clients do not.

This became concrete in PR #66. That PR scaffolded a browser frontend using
the SDK's `SSEClientTransport` — a spec-2024-11-05-compliant client. Live
testing failed in a specific and instructive way: `client.connect()` never
resolved, because the SDK waited for the tool-call response on the SSE
stream while chartgen was delivering it in the HTTP body. The client then
reconnected in a loop. The symptom was a frontend that looked like it was
"connecting" forever; the root cause was a transport mismatch that no
polish on either side would have fixed.

We must pick one spec-compliant transport and conform to it on both ends.

## Decision Drivers

- Correctness: work with the official MCP SDK without hand-rolled client
  code.
- Forward compatibility: pick the transport the MCP ecosystem is moving
  toward, not the one it is leaving.
- Minimum churn: prefer the option that requires the smallest server
  changes, since the frontend is still pre-v1.
- Compatibility with Claude.ai's remote-connector surface, which is our
  primary production client.

## Considered Options

**Stay on HTTP+SSE 2024-11-05 and make the server conformant.** Would
require implementing the bidirectional SSE response path server-side:
routing each `POST /mcp` request's response back through the per-client
SSE stream keyed by request ID. This is a meaningful server rewrite for a
transport the MCP spec has already deprecated in favour of Streamable HTTP.
More work, less forward-looking — rejected.

**Keep the hand-rolled bespoke transport and write a custom client.**
Works today with Claude.ai but not with any standard SDK. Every language
binding in the ecosystem would need a chartgen-specific shim. This is
vendor lock-in inverted — we become the vendor, and we pay the integration
cost forever. Rejected.

**Streamable HTTP 2025-03-26.** The current MCP transport spec. Single
endpoint (`POST /mcp`) that returns JSON either synchronously in the body
or as a server-initiated SSE stream, negotiated per-request via `Accept`
headers. Chosen — see below.

## Decision Outcome

Chosen option: **Streamable HTTP, spec 2025-03-26**, because chartgen's
existing `POST /mcp` handler already matches the synchronous-response path
of this transport, the frontend SDK ships a `StreamableHTTPClientTransport`
that we can adopt as-is, and it is the direction the MCP ecosystem is
moving.

chartgen's `initialize` response will advertise
`protocolVersion: "2025-03-26"`. The server needs minor polish to be fully
conformant: negotiate response mode (JSON body vs SSE stream) based on the
incoming `Accept` header; honour the `Mcp-Session-Id` header for session
continuity; and audit every response path — including error responses and
server-initiated notifications — to match the new spec's framing.

The frontend will use `StreamableHTTPClientTransport` from
`@modelcontextprotocol/sdk`. The OAuth 2.1 PKCE flow from PR #66 is
transport-agnostic and carries over unchanged, including the CSRF-check
fix.

## Consequences

**Positive.** Spec-compliant on both ends. Works with the official SDK
transports out of the box, which also unlocks third-party MCP clients
written in Python, Go, etc. Single HTTP endpoint simplifies reverse-proxy
and CDN configuration. Forward-compatible with newer MCP features
(resumable streams, session resumption) without another transport
migration.

**Negative.** Claude.ai remote-connector compatibility must be
re-validated. The expectation is that it works — Anthropic has announced
2025-03-26 support — but this must be confirmed in a live end-to-end test
before we cut over. A one-time server-side audit is required to confirm
that all response paths (successful tool calls, JSON-RPC errors, server
notifications) match the new spec's framing rules.

**Neutral.** Implementation is in a follow-up PR, not this one. This ADR
prescribes the direction; the code change lives alongside the
frontend-rewrite work.

## More Information

- [MCP Streamable HTTP specification (2025-03-26)](https://spec.modelcontextprotocol.io/specification/2025-03-26/)
- <!-- impl: #NNN -->
