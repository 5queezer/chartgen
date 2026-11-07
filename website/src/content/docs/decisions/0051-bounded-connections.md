---
title: "0051 — What one connection may cost"
description: "A small send buffer per socket, so many connections at once, and a stop to those that have stopped moving, because the memory of an answer nobody reads is charged to the server, not to any handler."
sidebar:
  order: 51
---

**Status:** Accepted
**Date:** 2026-11-07
**Amended by:** [ADR-0052](/chartgen/decisions/0052-indicator-cost/) (a call costs what its indicators draw and the bricks they run on, and keeps only its answer once built)

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
| After our last byte | Until the client has it all; reset if that stops moving for 30 s. A client hanging up its own side does not end this | Unread answers on connections closed by us, see below |
| Dropped with an answer unread | Reset | A client hanging up its side mid-answer, on which hyper drops the connection without closing it |
| Long-lived streams | 64, then `503` with `Retry-After`. A session's new stream shares the place of the one it replaces, and the replaced stream and its connection end | Streams taking every connection |
| Request body | 256 KiB by default | Half-sent bodies, which are read whole. A script is at most 64 KiB, or twice that escaped in JSON |
| Tool calls | 32,000 bars times series plus pixels / 512, until the answer has gone out | Charts and images being built and held. See below |
| A call waiting | 20 s, then `503` with `Retry-After: 5` | A connection held with no answer |
| A call larger than the budget | Refused at once, with the reason, `-32602` | One call alone at 5000 bars and 50 indicators peaked near 250 MiB, and one 16000 × 16000 image was killed outright |

A connection whose client has stopped reading costs at most about 730 KiB: 93 MiB for 128 of them.

**Closing.** A plain close of a socket with unread data leaves an orphan, which the kernel goes on trying
to send for minutes, still charged to the server, with the place in the cap already free. So answering
with `Connection: close` would have got round the cap. After the last byte a connection therefore keeps
its place until what the kernel holds for it is gone, and closes plainly then. While that shrinks it
waits, however slowly: a client at 5 KB/s and at 2 KB/s got all 143,061 bytes of a bundle. If it does not
move for 30 s the connection is reset, which drops what was not read.

A client hanging up its own side (`shutdown(SHUT_WR)`) says nothing of what it has read, and takes two routes.
With the answer already in the kernel, the wait above goes on, and it ends only when that is gone, or stops
moving, or the client resets. With the answer still being written, hyper treats it as an error and drops the
connection without closing it, so a connection dropped with data unsent is reset: the answer was cut off either
way, and no orphan stays behind. Before this, 600 such requests made 600 orphans in 3.5 s and got the server
killed, with no account.

A write has its own clock: 30 s with no write moving and the queue not shrinking. What was on its way when the
clock started can count as shrinking at the first look, so a client that stops in the middle of a large answer
is reset after about 60 s, measured at 60.0 to 60.4 s on a 1.6 MiB chart, and holds its connection and its share
of the budget that long. One that stops with everything in the kernel goes at 30 s.

What the kernel holds is read with `SIOCOUTQ`, on Linux only. On macOS and Windows, a connection is judged
by its writes alone, and a closing one gets 30 s and then a plain close. That can leave an orphan, but cuts
no reader, and production runs on Linux.

**Weighted tool calls.** A flat limit on calls cannot be right: sixteen chart calls at the largest size
are several times the container, and four at once would make the app's own grid wait. So a call costs what
it asks for: `bars × (indicators + 1)`, counting `panels` as well, at least 1000. A price-based chart
(`renko`, `kagi`, `point_figure`, `line_break`, `range`) adds the 20,000 bars it may read, up to the whole
budget, so that it takes its turn where the rest of it fits. `candles` and `heikin_ashi` add nothing: they are
built bar for bar. What is no tool call costs nothing and never waits. So does a request without a valid token,
which is answered without running anything: no account, no budget.

An image adds width × height / 512, for `png` and `both`. It is a bitmap of 3 bytes a pixel, whatever is drawn on
it, so that is 1.5 KiB a unit, the top of what charts measure. A limit per image alone would not do: at 1000 units
each, eight images of 4000 × 4000 at once, or 32 at the default 1920 × 1080, got the server killed. Priced, they
take turns. One image of 4000 × 4000 fits with few bars, at a peak of 55 MiB; one of 4096 × 4096 does not, and
over that is refused rather than clamped. The web app draws no images: it asks for `series`.

A chart measured 0.8 to 1.5 KiB per unit, the same on debug and release builds, from 500 bars and 4 series
to 5000 and 51. The budget of 32,000 holds 5000 bars with 5 indicators, or 1000 bars with 31. It fits the
app, which asks for 500 bars a call: a 4-chart grid at 10 indicators each comes to 22,000, and the 50 it
allows at most to 25,500, or the whole budget on a price-based chart. A call holds its share until its answer
has been handed over piece by piece, so hyper asks for the next piece only when it has room.

The cost does not see what a price-based chart builds. In `series`, indicators run on every bar built, up to
20,000 with a small box, though only `bars` are returned. On the sample source the default box makes about 150 to
3000 of them, depending on the style, and a renko box of 0.14% is refused for making over 20,000. One call at 0.15%,
50 of the heaviest indicators and 627 bars, the most it is charged for, peaked at 165 to 168 MiB and survived.

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

The budget was then set by the hardest case found at the time: large charts read in a tight loop from 8 threads,
with every other connection holding a half-sent body. On a release build the peak was about 130 MiB at 32,000, 216
at 48,000, and the 256 MiB limit at 64,000.

**A harder case was found after that, and it still kills the server, with a token.** Eight clients in a tight loop
on a chart of 627 bars and 50 of the heaviest indicators, with one answer unread and 100 half-sent bodies, reached a
peak of 221 MiB in the first round of 70 s, and got the server killed in the second. That is on plain `candles`, on
a build before the image and price-based changes here, and the same on `renko`. A small box gets there in the first
round: at 0.15%, 200 bars and 50 indicators, or 2909 bars and 10. At 500 bars and 50 on the default box, the app's
own most, `renko` and `range` survived two rounds at peaks of 217 and 240 MiB. So the budget bounds what the table
below shows, and not every call that fits it. A lower budget, or a cost that sees the indicators, has not been tried.

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
| The bundle, `Connection: close`, half-closed after 0.4 s or at once, 2400 requests, 5 runs | Not run | Not run | 115 to 156, no orphans |
| A map of 2.7 MB, `Connection: close`, half-closed after 0.4 s, 2400 requests | Not run | Not run | 146, no orphans |
| An asset that fits the send buffer, the same, 1440 requests | Not run | Not run | 57, no orphans |
| One image, 16000 × 16000 | Not run | Not run | Under 10, refused |
| Images of 4000 × 4000, 8 at once, three times | Not run | Not run | 61 |
| Images of 1920 × 1080, 64 at once, three times | Not run | Not run | 77 |
| One price-based chart, box 0.15%, 627 bars, 50 of the heaviest indicators | Not run | Not run | 165 to 168 |

The last seven are on the fixes here. Without them, on this listener, half-closing got the server killed by about
600 requests, and so did one image of 16000 × 16000, eight of 4000 × 4000 at once, or 32 of 1920 × 1080 at once.

### Consequences

- Good: none of the attacks in the table above ran the server out of memory by not reading, including a client
  that then hangs up its own side, on any route tried, on a directly reachable port. This is what was tried and
  not a proof: a token holder looping the largest calls still can, see above.
- Good: nothing here needs a setting.
- Bad: a tool user asking for 5000 bars with more than 5 indicators is refused, and told to ask for less.
  The schema still allows up to 50 indicators, as fewer bars can carry them. An image now counts as well: a
  5000-bar image takes at most 4 indicators at the default size.
- Bad: 128 connections can be held by anyone who can reach the port, and new ones then wait: an idle one
  for 120 s, one trickling a byte now and then for longer. For a flood of the kinds in the table above, that means
  waiting, not a killed server. Behind a tunnel the port is not reachable and a few connections carry all users.
- Bad: the cap counts a proxy's idle pool. A burst of 140 through a local Caddy with a pool of 200 for 90 s, as
  Traefik has, stayed at 5 connections, as answers are fast. But a burst of slow requests that really opens 128
  connections leaves them holding every place until the proxy closes them, 90 s later for Traefik, and a request
  past that waits. It takes about 64 requests in flight with the 64 streams open, far above the present load.
- Bad: a tool call can wait up to 20 s behind clients that stopped reading, and then comes back. One stopped in
  the middle of a large answer holds its share for about 60 s. A price-based chart takes most of the budget or
  all of it, so a grid of them takes turns.
- Bad: a stalled SSE stream with no chart ticks writes only its keepalive, which takes days to fill its
  buffer, so it can keep a place that long. At most 64.
- Bad: the proxy's own buffers towards a client are the proxy's to bound. With a proxy on the same host, it
  is the one to limit.
- Bad: the cost of a call is an estimate, for charts and images alone. Background chart ticks and other tools are
  not counted beyond the minimum, nor what price-based charts build, above. Price-based charts were not measured
  against a real provider, only the sample source's 5000 source bars.
- Bad: ending a replaced stream's connection ends it for whoever held it. Two clients sharing one token and one
  session id, or one token and no session id, replace each other. The web app has a session per tab.
- Bad: on macOS and Windows, or with another allocator than glibc, the lingering close and the allocator
  settings do less, as above. None of the numbers here apply to them.
- Neutral: answers from `/mcp` still say their length, and come out in pieces of 64 KiB.

## Testing

- Real sockets: `src/server/listener.rs`, for each deadline, the buffers as set, slow readers not being cut in
  a write, an idle wait or a close, a client hanging up its side, a keepalive stream not being cut, the cap, and
  what calls cost.
- Real binary: `tests/e2e.rs`, `test_connections_past_the_cap_wait_their_turn`,
  `test_streams_leave_room_for_everything_else`, `test_a_large_body_is_refused`,
  `test_an_unread_answer_does_not_stop_the_others`, `test_the_apps_charts_run_in_parallel`,
  `test_a_call_too_large_is_refused_with_the_reason`, `test_a_full_budget_makes_calls_wait_and_come_back`,
  `test_an_image_too_large_is_refused_with_the_reason`, `test_a_price_based_chart_with_many_indicators_takes_its_turn`,
  `test_a_client_hanging_up_its_side_leaves_no_orphan`, `test_a_replaced_stream_gives_back_its_connection`.
- Real browser: `web/tests/e2e/connection-limits.spec.ts`, a refused stream and a refused chart coming back.
- The memory tables above come from probes, not from the test suite: they need a cgroup. Not measured: the real
  Cloudflare, tunnel and Traefik path, or the Radxa (arm64).
