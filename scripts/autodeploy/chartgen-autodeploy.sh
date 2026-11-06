#!/usr/bin/env bash
# Pull-based deploy: poll GitHub for master's head, and when CI passed for a
# new SHA, trigger the Coolify deploy API. One run per systemd timer tick.
# See README.md and ADR-0037.
set -u
set +x # tracing would print COOLIFY_API_TOKEN; keep it off for the whole run

for bin in curl jq; do
  command -v "$bin" >/dev/null 2>&1 || { echo "chartgen-autodeploy: $bin is required but not installed" >&2; exit 2; }
done

: "${COOLIFY_URL:?COOLIFY_URL is not set}"
: "${COOLIFY_API_TOKEN:?COOLIFY_API_TOKEN is not set}"
: "${COOLIFY_APP_UUID:?COOLIFY_APP_UUID is not set}"
GITHUB_REPO=${GITHUB_REPO:-5queezer/chartgen}
STATE_FILE=${STATE_FILE:-${STATE_DIRECTORY:-/var/lib/chartgen-autodeploy}/last-sha}
API=https://api.github.com/repos/$GITHUB_REPO

log() { echo "chartgen-autodeploy: $*"; }

body=$(mktemp) || exit 1
trap 'rm -f "$body"' EXIT

# gh_get URL: GitHub GET into $body; prints the HTTP status ("000" on network error).
gh_get() {
  curl -sS --max-time 30 -o "$body" -w '%{http_code}' \
    -H 'User-Agent: chartgen-autodeploy' -H 'Accept: application/vnd.github+json' "$1" 2>/dev/null || true
}

code=$(gh_get "$API/commits/master")
if [ "$code" != 200 ]; then
  log "GitHub head lookup returned HTTP $code (rate limit or network); retrying next tick"
  exit 0
fi
sha=$(jq -r '.sha // empty' "$body")
[ -n "$sha" ] || { log "no sha in GitHub response; retrying next tick"; exit 0; }

if [ ! -f "$STATE_FILE" ]; then
  mkdir -p "$(dirname "$STATE_FILE")" && echo "$sha" >"$STATE_FILE"
  log "first run: recorded $sha without deploying"
  exit 0
fi
[ "$sha" = "$(cat "$STATE_FILE")" ] && exit 0

# Gate on the same thing the old deploy.yml did: the CI workflow's push run for this SHA.
code=$(gh_get "$API/actions/workflows/ci.yml/runs?head_sha=$sha&event=push&per_page=1")
if [ "$code" != 200 ]; then
  log "CI lookup for $sha returned HTTP $code; retrying next tick"
  exit 0
fi
status=$(jq -r '.workflow_runs[0].status // "none"' "$body")
conclusion=$(jq -r '.workflow_runs[0].conclusion // "none"' "$body")
if [ "$status" != completed ]; then
  log "CI for $sha not finished (status: $status); waiting"
  exit 0
fi
if [ "$conclusion" != success ]; then
  log "CI for $sha concluded '$conclusion'; not deploying"
  exit 0
fi

# The token goes to curl via a stdin config so it never appears in argv or in a trace.
code=$(printf 'header = "Authorization: Bearer %s"\n' "$COOLIFY_API_TOKEN" |
  curl -sS --max-time 60 -o "$body" -w '%{http_code}' -X POST -K - \
    "${COOLIFY_URL%/}/api/v1/deploy?uuid=$COOLIFY_APP_UUID&force=false" 2>/dev/null || true)

case $code in
  2??)
    if ! { mkdir -p "$(dirname "$STATE_FILE")" && echo "$sha" >"$STATE_FILE.tmp" && mv "$STATE_FILE.tmp" "$STATE_FILE"; }; then
      log "deployed $sha but could not write $STATE_FILE"
      exit 1
    fi
    log "triggered Coolify deploy for $sha (HTTP $code)"
    ;;
  *)
    log "Coolify deploy for $sha failed (HTTP $code); retrying next tick"
    exit 1
    ;;
esac
