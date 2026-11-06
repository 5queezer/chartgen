# Load test

A [Locust](https://locust.io) test that behaves like the web app: it opens the app,
loads charts, polls tails and the watchlist at the app's cadence, switches symbols,
pages back through history and holds a few push streams. It writes nothing: no alerts,
drawings, layouts or scripts.

Not for production without thought: read "Safety" first.

## Run

```sh
python3 -m venv ~/load-venv && ~/load-venv/bin/pip install locust   # outside the repo
mkdir -p ~/load-out && cd ~/load-out
~/load-venv/bin/locust -f <repo>/scripts/load/locustfile.py --headless \
    -H http://<host>:<port> --load-profile smoke --csv smoke
```

`-H` is required. Options also read the environment (`LOAD_PROFILE`, `LOAD_SYMBOLS`, ...);
`locust -f scripts/load/locustfile.py --help` lists them under "chartgen load test".
`--load-profile` (not `--profile`, which Locust owns) picks the shape:

| profile | shape |
|---|---|
| `smoke` | 1, 3, then 5 users over `--smoke-seconds` (120) |
| `ramp` | `--ramp-users` (5,10,20,40,80), `--step-seconds` (180) each; ends early if a stop condition trips |
| `soak` | `--soak-users` (10) for `--soak-seconds` (1800): watch memory growth |

## Safety

- **Data-source guard.** Right after authenticating, the test calls `get_quotes` for one
  symbol. Unless every answer has `source: sample` it exits 2 and sends no load, unless you
  pass `--real-data`. On a live server that one call is one Binance request, and a refused run
  still registers one OAuth client. The server has no cheaper way to say what it serves.
- **Stop conditions** end the run with exit code 3 and print why. Defaults, all parameters:
  error rate over 5% for 30 s (`--max-error-rate`, `--error-window`); p95 over 10 s for 60 s
  (`--max-p95-ms`, `--p95-window`); any 429/418 or an error text showing Binance's gate
  ("Binance rate limit", "HTTP 429") at once; 5 responses with status 5xx within 10 s
  (`--burst-5xx`, `--burst-window`). Rates need `--min-requests` (20) in the window.
- Exit codes: 0 clean, 1 final failure ratio above `--max-error-rate`, 2 refused or could not
  start, 3 stop condition.

## What it models

Weights and waits come from `web/src/lib/liveFeed.ts` and `web/src/queries.ts`.

- Per user: tail polls (bars 2, warmup 300) every 2 s for crypto, 15 s otherwise, doubling
  to 60 s on errors; `get_quotes` of the watchlist every 5 s. Tasks between 5 and 20 s apart:
  scroll back (6), switch symbol or interval (4), open the app cold (3), symbol search (2),
  PNG (`--png-weight`, 1; `--no-png` removes it).
- Auth: one OAuth client per run, registered once; each user gets its own token through the
  authorization-code flow with PKCE (token caps in `chart_push.rs` are per token). All of
  it is `authenticate()`. When sign-in (ADR-0044) lands, change that function only.
- Push: up to `--max-streams` (8) users hold one SSE stream and one `subscribe_chart`, and
  then stop polling, as the app does. A refused subscription (not pushable, push limit, feeds
  busy) falls back to polling. `PUSH` rows in the table are gaps between ticks, not request
  times, and do not count for the stop conditions. `--max-streams 0` turns push off.
  All users share one source address, so the server's 32-per-client cap applies.
- Heavy users (`HeavyUser`, `chart_patterns`, `volume_profile`): only if the server lists
  those indicators (`--heavy auto`); `--heavy off` removes them.

Not modelled: browser rendering, a hidden tab pausing polls, drawings, alerts and layouts,
the catch-up polls after a missed push, several tabs per person, assets served from a
browser cache, different source addresses (so the per-IP caps are the tightest case).
The load generator is one Locust process; use fewer than ~100 users per core.

## Radxa side

`watch.sh <container> out.csv [seconds]` samples container CPU and memory, restarts, OOM
kill and whole-machine load every 5 s. It only reads.
