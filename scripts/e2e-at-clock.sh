#!/usr/bin/env bash
# Run e2e specs against a sample server whose clock is shifted. Sample bars are a
# function of the clock, so a spec can pass for days and fail for an hour: this
# reaches those hours on demand. Only the server's clock moves, not the browser's.
# Linux with gcc only (an LD_PRELOAD shim). See guides/contributing.md.
#
# usage: scripts/e2e-at-clock.sh <offset: seconds, or 5h, 25d> [playwright args]
#   CHARTGEN_PORT=39315 WEB_PORT=35173 scripts/e2e-at-clock.sh 5h tests/e2e/smoke.spec.ts
set -euo pipefail

[ $# -ge 1 ] || { sed -n '7,8p' "$0" >&2; exit 2; }
case "$1" in
  *d) offset=$((${1%d} * 86400)) ;;
  *h) offset=$((${1%h} * 3600)) ;;
  *) offset=$(($1)) ;;
esac
shift

export CHARTGEN_PORT=${CHARTGEN_PORT:-9315} WEB_PORT=${WEB_PORT:-5173}
listening() { (exec 3<>"/dev/tcp/127.0.0.1/$1") 2>/dev/null; }
# Playwright reuses what it finds below, so a leftover server would be tested by mistake.
for port in "$CHARTGEN_PORT" "$WEB_PORT"; do
  if listening "$port"; then echo "port $port is in use" >&2; exit 1; fi
done

tmp=$(mktemp -d)
# The server's store: without CHARTGEN_DB it would be ~/.chartgen/chartgen.db, and every
# page load of a run would register a client there, dated by the shifted clock.
store=$(mktemp -d)
server=
trap '[ -z "$server" ] || kill "$server" 2>/dev/null; rm -rf "$tmp" "$store"' EXIT
trap 'exit 130' INT TERM # An unhandled signal would skip the EXIT trap.

gcc -shared -fPIC -O1 -x c -o "$tmp/shift.so" - -ldl <<'C'
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdlib.h>
#include <sys/time.h>
#include <time.h>
static long off(void) { const char *e = getenv("CLOCK_OFFSET_SECONDS"); return e ? atol(e) : 0; }
int clock_gettime(clockid_t id, struct timespec *t) {
  static int (*real)(clockid_t, struct timespec *);
  if (!real) real = dlsym(RTLD_NEXT, "clock_gettime");
  int r = real(id, t);
  if (r == 0 && (id == CLOCK_REALTIME || id == CLOCK_REALTIME_COARSE)) t->tv_sec += off();
  return r;
}
int gettimeofday(struct timeval *t, void *z) {
  static int (*real)(struct timeval *, void *);
  if (!real) real = dlsym(RTLD_NEXT, "gettimeofday");
  int r = real(t, z);
  if (r == 0 && t) t->tv_sec += off();
  return r;
}
time_t time(time_t *o) {
  struct timespec ts;
  clock_gettime(CLOCK_REALTIME, &ts);
  if (o) *o = ts.tv_sec;
  return ts.tv_sec;
}
C

cd "$(dirname "$0")/.."
cargo build --quiet
CHARTGEN_DATA_SOURCE=sample CHARTGEN_DB="$store/chartgen.db" \
  LD_PRELOAD="$tmp/shift.so" CLOCK_OFFSET_SECONDS=$offset \
  target/debug/chartgen --serve --port "$CHARTGEN_PORT" > "$tmp/server.log" 2>&1 &
server=$!
for _ in $(seq 60); do listening "$CHARTGEN_PORT" && break; sleep 1; done
listening "$CHARTGEN_PORT" || { cat "$tmp/server.log" >&2; exit 1; }
rm -rf "$tmp" # The server has loaded the shim; nothing left to clean up mid-run.

cd web
PW_REUSE_SERVER=1 npx playwright test "$@"
