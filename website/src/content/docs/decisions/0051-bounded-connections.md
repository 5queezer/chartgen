---
title: "0051 — What one connection may cost"
description: "A small send buffer per socket, so many connections at once, and a stop to those that have stopped moving, because the memory of an answer nobody reads is charged to the server, not to any handler."
sidebar:
  order: 51
---

**Status:** Accepted
**Date:** 2026-11-07

## Context and Problem Statement

A client can ask for something and never read the answer. The handler has finished by then. What it
returned sits in hyper's queue and in the kernel's send buffer of that socket, up to several MiB as the
kernel grows it, and all of that is charged to the server's cgroup. Nothing in the app bounded it.

The server runs in a 256 MiB container, and every token is in memory, so an OOM kill signs everyone out
and stops server alerts until a restart. On the plain server, in a 256 MiB scope:

- About 430 unread requests for the 660 KB web bundle killed it in about 2 s, with **no account**.
  `Connection: close` changed nothing.
- About 140 unread `generate_chart` calls of 2000 bars and six indicators killed it, with a token. A chart
  is built in memory: about 19 MiB at that size, about 55 MiB at the 5000-bar limit.

## Decision Drivers

- The bound must not depend on a proxy in front, and hold for every route, with or without an account.
- A healthy SSE stream is idle by design between its 30 s keepalives and must not be cut.
- Past a limit, clients should wait rather than fail.

## Considered Options

1. **Limits at the listener and on what is built per call.**
2. A global concurrency layer on the whole app. `GET /mcp` is a stream that lasts for hours, so it would
   hold a place all that time and lock everyone out.
3. Leave it to the proxy. It moves the memory to the proxy on the same small host: Caddy in front took
   1.5 GB for 1500 unread requests, and the server nothing.

## Decision Outcome

Option 1, in `src/server/listener.rs`.

| Limit | Value | What it bounds |
|---|---|---|
| Send buffer per socket | 128 KiB, doubled by Linux | Kernel memory for an unread answer |
| Receive buffer per socket | 48 KiB, doubled by Linux | Kernel memory for a request not read yet, which would otherwise grow to several MiB |
| Connections open at once | 128, and further accepts wait | All of the above, and hyper's queue of about 470 KiB per connection |
| A write that does not move | 30 s, then reset | A client that stopped reading. Also ends one slower than about 3 KB/s, since the kernel takes more only when about a third of the buffer has gone |
| Nothing either way | 120 s, then reset | Idle connections. Past the 90 s after which proxies close their own, so they find out first |
| After our last byte | 30 s for the client to hang up, then reset | Unread answers on connections closed by us, see below |
| Long-lived streams | 64, then `503` with `Retry-After` | Streams taking every connection. A tab refused one polls |
| Request body | 512 KiB by default | Half-sent bodies, which are read whole. A script is at most 64 KiB |
| Tool calls | 64,000 bars times series, until the answer has gone out, and the rest wait | Charts being built and held |

A connection whose client has stopped reading costs at most about 730 KiB: 93 MiB for 128 of them.

**Closing.** A plain close of a socket with unread data leaves an orphan, which the kernel goes on trying
to send for minutes, still charged to the server, with the place in the cap already free. So answering
with `Connection: close` would have got round the cap. After the last byte a connection therefore keeps
its place until the client hangs up, and one that has stalled is reset, which drops what it did not read.

**Weighted tool calls.** A flat limit on calls cannot be right: sixteen chart calls at the largest size
are several times the container, and four at once would make the app's own grid wait. So a call costs what
it asks for: `bars × (indicators + 1)`, measured at about 1.6 KiB per unit, and at least 1000. The budget
of 64,000 is about 100 MiB: one chart at the limit at a time, or 64 of the size the app usually asks for.
A price-based chart counts the 20,000 bars it may read. A call holds its share until its answer has been
handed over piece by piece, so hyper asks for the next piece only when it has room.

### Measured

A debug build, in `systemd-run --scope -p MemoryMax=256M`, with a client holding connections unread on a
4 KiB receive buffer.

| Flood | Before | Now |
|---|---|---|
| Web bundle, 1500 requests | Killed at about 430 | Alive: 107 MiB anon, 32 MiB socket |
| The same, `Connection: close` | Killed at about 530 | Alive: 105 MiB anon, 32 MiB socket |
| `generate_chart`, 2000 bars, 6 indicators | Killed at about 140 | Alive: 95 MiB anon |
| `generate_chart`, 5000 bars, 6 indicators | Killed | Alive: 52 MiB anon |
| Request heads that do not end, 1000 | Not run | Alive: 46 MiB anon |
| Bodies half sent, 1000 | Not run | Alive: 70 MiB anon |

### Consequences

- Good: no client can run the server out of memory by not reading, on any route, proxy or not.
- Good: nothing here needs a setting.
- Bad: 128 connections can be held by anyone who can reach the port, and new ones then wait: an idle one
  for 120 s, one trickling a byte now and then for longer. A flood means waiting, not a killed server. Behind
  a tunnel the port is not reachable and a few connections carry all users, so this needs the server's
  port open to the attacker.
- Bad: a tool call waits while others hold the budget, up to 30 s behind clients that stopped reading.
  That also holds for one chart at the limit against the rest.
- Bad: a stalled SSE stream with no chart ticks writes only its keepalive, which takes days to fill its
  buffer, so it can keep a place that long. At most 64 of them.
- Bad: the proxy's own buffers towards a client are the proxy's to bound. With a proxy on the same host, it
  is the one to limit.
- Bad: the cost of a call is an estimate, for charts alone. Background chart ticks and other tools are not
  counted beyond the minimum.
- Neutral: answers from `/mcp` still say their length, and come out in pieces of 64 KiB.

## Testing

- Real sockets: `src/server/listener.rs`, for each deadline, the buffers as set, a slow reader not being cut,
  a keepalive stream not being cut, and the cap.
- Real binary: `tests/e2e.rs`, `test_connections_past_the_cap_wait_their_turn`,
  `test_streams_leave_room_for_everything_else`, `test_a_large_body_is_refused`,
  `test_unread_tool_answers_are_limited_and_others_wait`.
- Real browser: `web/tests/e2e/connection-limits.spec.ts`, a tab refused a stream.
- The memory table above comes from a probe, not from the test suite: it needs a cgroup.
