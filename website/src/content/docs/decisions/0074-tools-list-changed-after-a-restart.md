---
title: "0074 — After a restart, a client hears that the tools may have changed"
description: "chartgen advertises tools.listChanged and keeps the session ids it gave out since it started. The first POST of a session from before the start is answered as an event stream that carries notifications/tools/list_changed before the answer, so a client re-reads its tools after a deploy without a reconnect."
sidebar:
  order: 74
---

**Status:** Accepted
**Date:** 2026-11-15
**Amends:** [ADR-0002](/chartgen/decisions/0002-mcp-transport-streamable-http/) (carries out
its "negotiate response mode (JSON body vs SSE stream) based on the incoming `Accept` header" for
one case: the first POST of a session from before a restart; every other POST stays JSON),
[ADR-0021](/chartgen/decisions/0021-realtime-chart-push/) (besides scoping chart push, a session
id is now remembered: the server keeps the ids it gave out since it started). Neither decision is
replaced: no earlier ADR says a POST is always answered as JSON, or that the server keeps no
session state.

## Context and Problem Statement

After a deploy that adds a tool, Hermes (the agent that reads chartgen over MCP) keeps the tool
list it read before. `get_market_context` was missing for it after #174 until its MCP server was
toggled by hand. Hermes's client handles `notifications/tools/list_changed`: it re-reads
`tools/list` and registers the difference. chartgen never sends it.

chartgen accepts any `Mcp-Session-Id` and keeps no session state, so a client's session survives a
restart: it never initializes again, and never reads the tools again.

**How the client behaves, proven** with the MCP Python SDK 2.3.0 that Hermes runs, against a local
chartgen:

- It opens a GET stream right after `initialize`, for the server's own messages.
- **When the server is down for 0.5 s,** the stream is open again after 1 s.
- **When it is down for 10 s,** the stream fails twice and is given up for good: "max reconnection
  attempts (2) exceeded". It is not opened again for the rest of the session.
- Either way the old session keeps working: calls and pings go on as before.

Hermes's own log agrees. It sent one GET per connection all day (10:05, 13:41, 18:58, 20:35, 22:14,
23:00 on 11-14) and none after the 22:40 deploy. A notice sent only when a GET stream opens would
have reached nobody that day.

## Decision Drivers

- No change on the client, and no manual step after a deploy.
- No error on the way: a call made just after a deploy must still be answered. Hermes does not retry
  a write tool after a session error; it reports the outcome as uncertain.
- Cheap: one extra `tools/list` per client per restart.
- Clients that keep no session (the web app sends no `Mcp-Session-Id`) and clients that take only
  JSON are unaffected.

## Considered Options

1. **The first POST of a session from before the start is answered as an event stream, with
   `notifications/tools/list_changed` before the answer** (chosen).
2. Send it on every GET stream that opens. It never reaches a client whose stream was given up during
   the deploy, and production showed no reopened stream after a deploy.
3. Answer an unknown session id with `404`, which per the spec makes the client initialize again
   and so re-list. Every client gets an error on its first call after a deploy, and Hermes turns
   that into "outcome uncertain" for a write tool instead of retrying it.

## Decision Outcome

- `initialize` advertises `capabilities.tools.listChanged: true`.
- The server keeps, in memory, **the session ids it gave out since it started** (on `initialize`
  without one), and the older ones it has already told. At most 10 000; then it starts over, and a
  client told twice re-reads its tools once more.
- **A POST that carries an id the server does not keep** was given out before it started, by a build
  that may have had other tools. If its `Accept` takes `text/event-stream`, and its answer is a
  `200` JSON answer to a call other than `initialize`, the answer goes out as an event stream:

      event: message
      data: {"jsonrpc":"2.0","method":"notifications/tools/list_changed"}

      event: message
      data: <the answer>

  The id is then kept, so this happens once per session per start.
- Anything else (no session id, a client that takes only JSON, an `initialize`, an answer that is
  not `200`, a POST of notifications only, answered `202`) is answered as before; such a session is
  told with its next call that can be.

The first request a client sends after a deploy, which for Hermes is its keepalive ping within a
few minutes if no tool call comes first, then carries the notice. The client re-reads its tools.

### Consequences

- Good: new tools reach a connected client after any deploy, however long the server was away, with
  no error and no reconnect.
- Good: no client change; it is the spec's notification on a transport the spec allows for it.
- Neutral: after every restart, each client with an old session re-reads its tools once, whether
  they changed or not. The server cannot tell what an earlier build listed, and a re-read is cheap.
- Neutral: the kept ids are memory only, a few hundred kilobytes at the most.
- Bad: a client that is told and then loses the answer (a dropped connection) is not told again
  until the next restart. It still has its old list, as today.

## Validation

- Unit tests: an unknown session's first answer is an event stream with the notice before the
  answer, and its second is JSON; the server's own session, a JSON-only client and an `initialize`
  with an old id get JSON; `initialize` advertises `listChanged`.
- The SDK proof above, run again on this build: with the server down for 10 s and the GET stream
  given up, the first `ping` after the restart delivered `tools/list_changed` to the client's message
  handler, and the ping and the `tools/list` after it were answered. The same probe on the build
  before saw no notice.
- After the deploy: Hermes's log shows "received tools/list_changed notification" for chartgen and a
  new "registered N tool(s)", without a toggle.
