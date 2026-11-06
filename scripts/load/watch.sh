#!/usr/bin/env bash
# Read-only sampler for the machine that runs chartgen. One CSV row every INTERVAL
# seconds: container CPU and memory, restarts, OOM kill, whole-machine load and memory.
#
#   watch.sh <container> [out.csv] [interval-seconds]
#
# Needs only `docker stats` / `docker inspect` (set DOCKER=podman to use podman).
# Stop with Ctrl-C or `timeout`. It changes nothing on the host.
set -u
CONTAINER=${1:?usage: watch.sh <container> [out.csv] [interval-seconds]}
OUT=${2:-watch.csv}
INTERVAL=${3:-5}
DOCKER=${DOCKER:-docker}

[ -f "$OUT" ] || echo "ts,cpu_pct,mem_used,mem_limit,restarts,oom_killed,status,load1,load5,mem_avail_mb" >"$OUT"
while true; do
  ts=$(date -u +%FT%TZ)
  # "12.3%,40.1MiB / 256MiB"
  stats=$("$DOCKER" stats --no-stream --format '{{.CPUPerc}},{{.MemUsage}}' "$CONTAINER" 2>/dev/null || echo ",")
  cpu=${stats%%,*}; mem=${stats#*,}
  used=${mem%% / *}; limit=${mem#* / }
  state=$("$DOCKER" inspect -f '{{.RestartCount}},{{.State.OOMKilled}},{{.State.Status}}' "$CONTAINER" 2>/dev/null || echo ",,gone")
  read -r l1 l5 _ </proc/loadavg
  avail=$(awk '/MemAvailable/ {printf "%d", $2/1024}' /proc/meminfo)
  echo "$ts,${cpu%\%},$used,$limit,$state,$l1,$l5,$avail" >>"$OUT"
  sleep "$INTERVAL"
done
