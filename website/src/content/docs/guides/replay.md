---
title: Replay over real history
description: Write replay fixtures from real Binance 1m history with chartgen replay-fixtures, and run a private replay server on them, so an agent can be tested on a past quarter in hours.
---

A [replay server](/chartgen/guides/setups/#replay) answers chart, indicator, quote and setup
tools as of a time each call names, from 1m fixture bars. `chartgen replay-fixtures` writes those
fixtures from real Binance history, so an agent can trade a past quarter on paper in an afternoon
instead of waiting weeks for live setups to close. The decision is
[ADR-0068](/chartgen/decisions/0068-replay-over-real-history/).

## Write the fixtures

```bash
chartgen replay-fixtures --symbols BTCUSDT,ETHUSDT \
  --from 2026-08-01 --to 2026-10-31 --out fixtures/2026-q3
```

| Flag | Meaning |
|------|---------|
| `--symbols` | Binance spot symbols, comma-separated. |
| `--from` | First day of the window, UTC (`YYYY-MM-DD`). |
| `--to` | Last day, UTC, included. It must have ended: a fixture holds closed bars only. |
| `--out` | Directory for one `<SYMBOL>.json` per symbol. Created if missing. |
| `--shift-days` | Move every timestamp by this many days (negative moves back). Default `0`. |

It prints a line per symbol: bars, minutes missing, gaps over 5 minutes, size and time, then
each gap in real time (the first 20).

- **Size bound.** A run may write at most **460 symbol-days**: 5 symbols over a 92-day quarter, or
  2 symbols over 230 days, and so on. A replay server keeps every 1m bar in memory, about 100
  bytes each, so that is about 66 MB. A larger window is refused before any request. On disk a
  fixture is about 56 bytes per bar, about 7.5 MB per symbol-quarter.
- **Binance load.** One request per 1000 bars, about 130 per symbol-quarter, at most 10 a second,
  through the same gate as the server. That is at most a fifth of the IP's weight a minute, so a
  live server on the same address keeps working. Run it where the replay will run (your PC), not
  on the live server.
- **Interrupted runs.** Each file records the window it was built for and is written whole or
  not at all. Run the same command again: symbols already written are kept with no request, and
  the rest are read. A file of another window is not overwritten; choose another `--out` or
  remove it.
- **Gaps.** A minute Binance has no kline for (an outage, or no trade on a small symbol) stays
  missing. The replay builds each longer bar from the minutes it has, and a bar with no minute at
  all is not there, as on Binance. If the window's first minutes are missing, the first bar of
  each longer interval is left out.
- **Shift.** `--shift-days` hides the real dates from the agent. Prices still show the market
  regime. A multiple of 7 keeps weekends on weekends and weekly bars the same; any shift changes
  the monthly bars.

Every `*.json` in the directory is loaded, so keep one window per directory. Record a hash of
the files with each run (`sha256sum fixtures/2026-q3/*.json`), so the run can be checked later.

## Run a private replay server

:::caution[Never public]
A replay server refuses sign-in, and with sign-in off anyone who can reach it can register a
client and get a token without a person. Anyone can then move its one clock to the end of the
window, which spoils the run. Never route it through Cloudflare or any public proxy.
`chartgen --serve` listens on every interface and has no option to bind one address, so keep it
private with where it runs: a container whose port is published on the Tailscale address only,
or a container on an internal network.
:::

**On your PC, over Tailscale.** The published image is arm64 only, so build it on x86
(`docker build -t chartgen .` in the repository), then publish the port on the Tailscale
address alone:

```bash
TS_IP=$(tailscale ip -4)
podman run --rm --name chartgen-replay \
  -v "$PWD/fixtures/2026-q3:/fixtures:ro,Z" \
  -e CHARTGEN_REPLAY_FIXTURES=/fixtures \
  -e CHARTGEN_BASE_URL="http://$TS_IP:9316" \
  -p "$TS_IP:9316:9315" \
  chartgen
```

**On a small arm64 host, next to a live server.** A second container on an internal network,
with no published port, which the agent's container joins. Up to the size bound above it fits
beside the live services.

```bash
docker network create --internal chartgen-replay
docker run -d --name chartgen-replay --network chartgen-replay \
  -v /srv/replay/2026-q3:/fixtures:ro \
  -e CHARTGEN_REPLAY_FIXTURES=/fixtures \
  -e CHARTGEN_BASE_URL=http://chartgen-replay:9315 \
  ghcr.io/5queezer/chartgen:master
docker network connect chartgen-replay <agent container>
```

A bare `chartgen --serve` with `CHARTGEN_REPLAY_FIXTURES` set is reachable from your LAN unless
a firewall admits the port on the Tailscale interface only.

The agent connects as a [headless client](/chartgen/reference/oauth/#connecting-a-headless-agent).
With sign-in off it needs no person: it registers, follows `/authorize` and exchanges the code.

- **One process is one run.** It has one clock, which only moves forward. To run again, restart
  it. To run two evaluations at once, start two servers on two ports.
- **Export before stopping.** Setups live in the process's memory. Read `list_setups` and
  `get_setup` for each before the server stops.
- **Every call needs `as_of`.** The tools that cannot be answered from 1m bars are refused; see
  [Replay](/chartgen/guides/setups/#replay).

## Look-ahead outside chartgen

chartgen cuts every answer at `as_of`. It cannot cut what the agent reaches some other way:

- **The agent's other tools.** Web search, news and a macro calendar answer for today. Give a
  replay session the replay server as its only MCP server. A macro calendar needs a fixture of
  past releases for the window, answered at `as_of`, on the agent's side.
- **The model's memory.** A model trained past the window may know what the price did next.
  Prefer windows after the model's training cutoff. The shift hides dates, not price levels.
  Record the model with every run.
