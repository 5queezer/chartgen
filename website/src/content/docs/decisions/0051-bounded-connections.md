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
  is built in memory: about 20 MiB at that size, about 50 MiB at 5000 bars and six indicators.

## Decision Drivers

- The bound must not depend on a proxy in front, and hold for every route, with or without an account.
- A healthy SSE stream is idle by design between its 30 s keepalives and must not be cut, nor a slow
  client that is still reading.
- Past a limit, clients should wait or come back rather than fail.
- The app's own use must not notice: a grid of charts runs side by side.

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
| A client not moving | 30 s with a write waiting, or 120 s with nothing either way, then reset | A client that stopped reading. What the kernel still holds counts as moving when it shrinks (Linux), so a slow reader is not one |
| After our last byte | Until the client has it all; reset if that stops moving for 30 s | Unread answers on connections closed by us, see below |
| Long-lived streams | 64, then `503` with `Retry-After`. A session's new stream shares the place of the one it replaces | Streams taking every connection |
| Request body | 256 KiB by default | Half-sent bodies, which are read whole. A script is at most 64 KiB, or twice that escaped in JSON |
| Tool calls | 32,000 bars times series, until the answer has gone out | Charts being built and held. See below |
| A call waiting | 20 s, then `503` with `Retry-After: 5` | A connection held with no answer |
| A call larger than the budget | Refused at once, with the reason | One call alone at 5000 bars and 50 indicators peaked near 250 MiB |

A connection whose client has stopped reading costs at most about 730 KiB: 93 MiB for 128 of them.

**Closing.** A plain close of a socket with unread data leaves an orphan, which the kernel goes on trying
to send for minutes, still charged to the server, with the place in the cap already free. So answering
with `Connection: close` would have got round the cap. After the last byte a connection therefore keeps
its place until what the kernel holds for it is gone, and closes plainly then. While that shrinks it
waits, however slowly: a client at 5 KB/s and at 2 KB/s got all 143,061 bytes of a bundle. If it does not
move for 30 s the connection is reset, which drops what was not read.

What the kernel holds is read with `SIOCOUTQ`, on Linux only. On macOS and Windows, a connection is judged
by its writes alone, and a closing one gets 30 s and then a plain close. That can leave an orphan, but cuts
no reader, and production runs on Linux.

**Weighted tool calls.** A flat limit on calls cannot be right: sixteen chart calls at the largest size
are several times the container, and four at once would make the app's own grid wait. So a call costs what
it asks for: `bars × (indicators + 1)`, counting `panels` as well, at least 1000. A price-based chart
(`renko`, `kagi`, `point_figure`, `line_break`, `range`) adds the 20,000 bars it may read. `candles` and
`heikin_ashi` add nothing: they are built bar for bar. What is no tool call costs nothing and never waits.
So does a request without a valid token, which is answered without running anything: no account, no budget.

A chart measured 0.8 to 1.5 KiB per unit, the same on debug and release builds, from 500 bars and 4 series
to 5000 and 51. The budget of 32,000 holds 5000 bars with 5 indicators, or 1000 bars with 31. It fits the
app, which asks for 500 bars a call: a 4-chart grid at 10 indicators each comes to 22,000, and the 50 it
allows at most to 25,500. A call holds its share until its answer has been handed over piece by piece, so
hyper asks for the next piece only when it has room.

The web app comes back from a `503` by itself: a call after `Retry-After`, up to 6 tries, and a refused
stream for as long as the tab wants it, so that it returns to pushed data. Its charts show no error for it.

**What the allocator keeps.** This was most of the problem, and no limit on live data touches it. After a
round of large charts glibc held about 180 MiB with nothing in use, and the next round was killed. Measured
on the review's attack:

| Setting | Result |
|---|---|
| Defaults, budget 64,000 | Killed in round 2 |
| `MALLOC_TRIM_THRESHOLD_` and `MALLOC_MMAP_THRESHOLD_` at 128 KiB | Killed in round 2 |
| 2 arenas (`M_ARENA_MAX`) | Alive, about 180 MiB held with nothing in use |
| 2 arenas and `malloc_trim` every 30 s | Killed in round 2 to 4 |
| 2 arenas and `malloc_trim` within 2 s of a tool call or connection ending | Alive, about 35 MiB held with nothing in use |

So the server sets the last of these at startup, on Linux with glibc, and leaves the allocator alone elsewhere.

The budget was then set by the hardest case found: large charts read in a tight loop from 8 threads, with
every other connection holding a half-sent body. On a release build the peak was about 130 MiB at 32,000, 216
at 48,000, and the 256 MiB limit at 64,000.

### Measured

Peak of the cgroup under `systemd-run --scope -p MemoryMax=256M`, in MiB, each on a fresh process, with a client
that does not read on a 4 KiB receive buffer. The plain server was killed by the attacks marked so. Peaks include
reclaimable file pages.

| Attack | Plain server | Debug | Release |
|---|---|---|---|
| Web bundle, 1500 requests | Killed at about 430 | 140 | 163 |
| The same, `Connection: close` | Killed at about 530 | 138 | 175 |
| A smaller asset that fits the send buffer, `Connection: close`, 1500 | Not run | 57 | 64 |
| `generate_chart`, 2000 bars, 6 indicators, 600 requests | Killed at about 140 | 51 | 102 |
| `generate_chart`, 5000 bars, 5 indicators, 300 requests | Killed | 38 | 65 |
| Request heads that do not end, 1000 | Not run | 52 | 68 |
| Bodies half sent, 1000 | Not run | 65 | 65 |
| The budget full of unread answers, every other connection a half-sent body, 200 more waiting, twice | Killed in round 2 | 104 | 59 |
| The same, filled exactly, three times | Not run | 63 | 64 |
| Charts of the largest size, then 25 and 50 indicators (refused), twice | Killed | 45 | 42 |
| Large charts in a loop, 8 threads, half-sent bodies held | Not run | 112 | 113 |
| The app's charts in a loop, 16 threads, half-sent bodies held | Not run | 87 | 90 |

### Consequences

- Good: no client can run the server out of memory by not reading, on any route, proxy or not, in what was
  tried above.
- Good: nothing here needs a setting.
- Bad: a tool user asking for 5000 bars with more than 5 indicators is refused, and told to ask for less.
  The schema still allows up to 50 indicators, as fewer bars can carry them.
- Bad: 128 connections can be held by anyone who can reach the port, and new ones then wait: an idle one
  for 120 s, one trickling a byte now and then for longer. A flood means waiting, not a killed server. Behind
  a tunnel the port is not reachable and a few connections carry all users.
- Bad: the cap counts a proxy's idle pool. A burst of 140 through a local Caddy with a pool of 200 for 90 s, as
  Traefik has, stayed at 5 connections, as answers are fast. But a burst of slow requests that really opens 128
  connections leaves them holding every place until the proxy closes them, 90 s later for Traefik, and a request
  past that waits. It takes about 64 requests in flight with the 64 streams open, far above the present load.
- Bad: a tool call can wait up to 20 s behind clients that stopped reading, and then comes back. A price-based
  chart takes most of the budget, so a grid of them takes turns.
- Bad: a stalled SSE stream with no chart ticks writes only its keepalive, which takes days to fill its
  buffer, so it can keep a place that long. At most 64.
- Bad: the proxy's own buffers towards a client are the proxy's to bound. With a proxy on the same host, it
  is the one to limit.
- Bad: the cost of a call is an estimate, for charts alone. Background chart ticks and other tools are not
  counted beyond the minimum. Price-based charts could not be measured against a real 20,000-bar source: the
  sample source has fewer.
- Bad: on macOS and Windows, or with another allocator than glibc, the lingering close and the allocator
  settings do less, as above. None of the numbers here apply to them.
- Neutral: answers from `/mcp` still say their length, and come out in pieces of 64 KiB.

## Testing

- Real sockets: `src/server/listener.rs`, for each deadline, the buffers as set, slow readers not being cut in
  a write, an idle wait or a close, a keepalive stream not being cut, the cap, and what calls cost.
- Real binary: `tests/e2e.rs`, `test_connections_past_the_cap_wait_their_turn`,
  `test_streams_leave_room_for_everything_else`, `test_a_large_body_is_refused`,
  `test_an_unread_answer_does_not_stop_the_others`, `test_the_apps_charts_run_in_parallel`,
  `test_a_call_too_large_is_refused_with_the_reason`, `test_a_full_budget_makes_calls_wait_and_come_back`.
- Real browser: `web/tests/e2e/connection-limits.spec.ts`, a refused stream and a refused chart coming back.
- The memory tables above come from probes, not from the test suite: they need a cgroup. Not measured: the real
  Cloudflare, tunnel and Traefik path, or the Radxa (arm64).
