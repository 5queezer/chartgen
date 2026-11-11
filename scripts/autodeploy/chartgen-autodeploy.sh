#!/usr/bin/env bash
# Pull-based deploy: poll GitHub for master's head, and when CI passed for a
# new SHA and its image is in the registry, point the Coolify application at
# that image and trigger the deploy API. One run per systemd timer tick.
#
#   chartgen-autodeploy.sh               one tick
#   chartgen-autodeploy.sh deploy SHA    deploy the image of SHA now
#   chartgen-autodeploy.sh rollback      deploy the image that ran before the one this script put out last
#
# See README.md, ADR-0037 and ADR-0060.
set -u
set +x # tracing would print COOLIFY_API_TOKEN; keep it off for the whole run

for bin in curl jq flock; do
  command -v "$bin" >/dev/null 2>&1 || { echo "chartgen-autodeploy: $bin is required but not installed" >&2; exit 2; }
done

: "${COOLIFY_URL:?COOLIFY_URL is not set}"
: "${COOLIFY_API_TOKEN:?COOLIFY_API_TOKEN is not set}"
: "${COOLIFY_APP_UUID:?COOLIFY_APP_UUID is not set}"
GITHUB_REPO=${GITHUB_REPO:-5queezer/chartgen}
CHARTGEN_IMAGE=${CHARTGEN_IMAGE:-ghcr.io/5queezer/chartgen}
STATE_FILE=${STATE_FILE:-${STATE_DIRECTORY:-/var/lib/chartgen-autodeploy}/last-sha}
# What this script last put out, by the timer or by hand, and what ran before that.
DEPLOYED_FILE=$(dirname "$STATE_FILE")/deployed-sha
PREVIOUS_FILE=$(dirname "$STATE_FILE")/previous-sha
# Left by a command by hand that could not ask GitHub for master's head: the next
# tick that can ask leaves that head alone.
HAND_FILE=$(dirname "$STATE_FILE")/by-hand
LOCK_FILE=$(dirname "$STATE_FILE")/lock
LOCK_WAIT_SECS=${LOCK_WAIT_SECS:-120}
API=https://api.github.com/repos/$GITHUB_REPO
REGISTRY=${CHARTGEN_IMAGE%%/*}
IMAGE_PATH=${CHARTGEN_IMAGE#*/}

log() { echo "chartgen-autodeploy: $*"; }

body=$(mktemp) || exit 1
trap 'rm -f "$body"' EXIT

# gh_get URL: GitHub GET into $body; prints the HTTP status ("000" on network error).
gh_get() {
  curl -sS --max-time 30 -o "$body" -w '%{http_code}' \
    -H 'User-Agent: chartgen-autodeploy' -H 'Accept: application/vnd.github+json' "$1" 2>/dev/null || true
}

# write_file FILE VALUE: atomic, through a temporary file of its own beside it
write_file() {
  local tmp
  mkdir -p "$(dirname "$1")" && tmp=$(mktemp "$1.XXXXXX") && echo "$2" >"$tmp" && mv "$tmp" "$1"
}

# One run at a time, from reading the state to writing it. The timer's service and a
# command by hand are separate processes: unlocked, one could set its tag between the
# other's tag and deploy, and Coolify would deploy one image while the other is
# recorded. The lock is held until the script exits.
#   take_turn tick: another run under way means this tick has nothing to do.
#   take_turn hand: wait for it, and fail loudly rather than do nothing quietly.
take_turn() {
  mkdir -p "$(dirname "$LOCK_FILE")" || { log "could not make the state directory"; exit 1; }
  exec 9>"$LOCK_FILE" || { log "could not open $LOCK_FILE"; exit 1; }
  if [ "$1" = tick ]; then
    flock -n 9 || { log "another run is under way; this tick does nothing"; exit 0; }
  else
    flock -w "$LOCK_WAIT_SECS" 9 || { log "another run still held the lock after $LOCK_WAIT_SECS s; nothing was deployed. Try again"; exit 1; }
  fi
}

# image_status SHA: prints the HTTP status of the image's manifest in the registry,
# read as anyone can read a public package. 200: it is there. 404: not yet.
image_status() {
  local code token
  code=$(curl -sS --max-time 30 -o "$body" -w '%{http_code}' \
    "https://$REGISTRY/token?service=$REGISTRY&scope=repository:$IMAGE_PATH:pull" 2>/dev/null || true)
  if [ "$code" != 200 ]; then
    echo "token-$code"
    return
  fi
  token=$(jq -r '.token // empty' "$body" 2>/dev/null)
  [ -n "$token" ] || { echo token-empty; return; }
  printf 'header = "Authorization: Bearer %s"\n' "$token" |
    curl -sS --max-time 30 -o /dev/null -w '%{http_code}' -I -K - \
      -H 'Accept: application/vnd.oci.image.index.v1+json' \
      -H 'Accept: application/vnd.oci.image.manifest.v1+json' \
      -H 'Accept: application/vnd.docker.distribution.manifest.list.v2+json' \
      -H 'Accept: application/vnd.docker.distribution.manifest.v2+json' \
      "https://$REGISTRY/v2/$IMAGE_PATH/manifests/$1" 2>/dev/null || true
}

# coolify METHOD PATH [JSON]: prints the HTTP status. The token goes to curl via a
# stdin config so it never appears in argv or in a trace.
coolify() {
  local data=()
  [ $# -ge 3 ] && data=(-H 'Content-Type: application/json' --data "$3")
  printf 'header = "Authorization: Bearer %s"\n' "$COOLIFY_API_TOKEN" |
    curl -sS --max-time 60 -o "$body" -w '%{http_code}' -X "$1" -K - "${data[@]}" \
      "${COOLIFY_URL%/}/api/v1$2" 2>/dev/null || true
}

# deploy_image SHA: point the application at the image of SHA and deploy it.
# Returns 0 when Coolify accepted both calls, which is "triggered", not "healthy".
deploy_image() {
  local code json kind
  # Only an application that runs a prebuilt image. On one that builds from a
  # Dockerfile the tag would be ignored and the deploy would build on this host:
  # the build ADR-0060 took off it. Anything but a clear answer refuses.
  code=$(coolify GET "/applications/$COOLIFY_APP_UUID")
  case $code in
    2??) ;;
    *)
      log "could not read the Coolify application (HTTP $code); not deploying"
      return 1
      ;;
  esac
  kind=$(jq -r '.build_pack // "unknown"' "$body" 2>/dev/null)
  if [ "$kind" != dockerimage ]; then
    log "the Coolify application $COOLIFY_APP_UUID is not a Docker Image application (build_pack: ${kind:-unknown}); not deploying. A deploy would build on this host. Check COOLIFY_APP_UUID, see README.md"
    return 1
  fi
  json=$(jq -cn --arg name "$CHARTGEN_IMAGE" --arg tag "$1" \
    '{docker_registry_image_name: $name, docker_registry_image_tag: $tag}')
  code=$(coolify PATCH "/applications/$COOLIFY_APP_UUID" "$json")
  case $code in
    2??) ;;
    *)
      log "Coolify did not take the image tag $1 (HTTP $code)"
      return 1
      ;;
  esac
  code=$(coolify POST "/deploy?uuid=$COOLIFY_APP_UUID&force=false")
  case $code in
    2??) ;;
    *)
      log "Coolify deploy for $1 failed (HTTP $code)"
      return 1
      ;;
  esac
}

# record_deploy SHA: SHA is what runs now, and what ran until now is what a rollback
# goes back to. Until this script has put something out, that is the timer's last SHA.
record_deploy() {
  local before=
  if [ -s "$DEPLOYED_FILE" ]; then
    before=$(cat "$DEPLOYED_FILE")
  elif [ -s "$STATE_FILE" ]; then
    before=$(cat "$STATE_FILE")
  fi
  if [ -n "$before" ] && [ "$before" != "$1" ]; then
    write_file "$PREVIOUS_FILE" "$before" || return 1
  fi
  write_file "$DEPLOYED_FILE" "$1"
}

# A command by hand is the operator's word on what runs, so the timer must not put
# master's present head out over it: that head may be the very commit that was just
# rolled back. The timer's mark is moved to master's head as it is now, without
# deploying it. The next commit on master deploys as usual. If GitHub cannot be asked
# now, the next tick that can ask does the same.
leave_head_alone() {
  local code head
  code=$(gh_get "$API/commits/master")
  head=
  [ "$code" = 200 ] && head=$(jq -r '.sha // empty' "$body" 2>/dev/null)
  if [ -n "$head" ]; then
    rm -f "$HAND_FILE"
    write_file "$STATE_FILE" "$head"
  else
    write_file "$HAND_FILE" 1
  fi
}

# By hand: deploy a published image, whatever master is at.
manual() {
  local sha=$1 status
  status=$(image_status "$sha")
  if [ "$status" != 200 ]; then
    log "no image $CHARTGEN_IMAGE:$sha to deploy (registry answered $status)"
    exit 1
  fi
  deploy_image "$sha" || exit 1
  if ! record_deploy "$sha" || ! leave_head_alone; then
    log "deployed $sha by hand but could not record it in $(dirname "$STATE_FILE"); the timer may deploy master's head over it"
    exit 1
  fi
  log "triggered Coolify deploy of $CHARTGEN_IMAGE:$sha by hand; the timer leaves master's present head alone and deploys the next commit"
}

case ${1-} in
  '') ;;
  deploy)
    # A full commit SHA and nothing else: a name like "master" is a tag that moves.
    if ! [[ ${2-} =~ ^[0-9a-f]{40}$ ]]; then
      echo "usage: $0 deploy SHA (the full commit SHA, 40 hex characters)" >&2
      exit 2
    fi
    take_turn hand
    manual "$2"
    exit 0
    ;;
  rollback)
    take_turn hand
    [ -s "$PREVIOUS_FILE" ] || { log "no previous deploy is recorded in $PREVIOUS_FILE; use: deploy SHA"; exit 1; }
    manual "$(cat "$PREVIOUS_FILE")"
    exit 0
    ;;
  *)
    echo "usage: $0 [deploy SHA | rollback]" >&2
    exit 2
    ;;
esac

take_turn tick
code=$(gh_get "$API/commits/master")
if [ "$code" != 200 ]; then
  log "GitHub head lookup returned HTTP $code (rate limit or network); retrying next tick"
  exit 0
fi
sha=$(jq -r '.sha // empty' "$body")
[ -n "$sha" ] || { log "no sha in GitHub response; retrying next tick"; exit 0; }

if [ ! -s "$STATE_FILE" ]; then
  write_file "$STATE_FILE" "$sha" || { log "could not write $STATE_FILE"; exit 1; }
  log "first run: recorded $sha without deploying"
  exit 0
fi
last=$(cat "$STATE_FILE")
if [ -e "$HAND_FILE" ]; then
  # A command by hand came since the last tick and could not ask GitHub then.
  if [ "$sha" != "$last" ]; then
    write_file "$STATE_FILE" "$sha" || { log "could not write $STATE_FILE"; exit 1; }
    log "not deploying $sha: an image was deployed by hand, and master's head of that time is left alone. The next commit deploys as usual. To put this one out: deploy $sha"
  fi
  rm -f "$HAND_FILE"
  exit 0
fi
[ "$sha" = "$last" ] && exit 0

# The push run of CI for this SHA must be green. On master that is the run that builds
# the image, so the image check below usually agrees. It does not always: CI run by
# hand on master's head builds an image too, and that one is not deployed while the
# push run is red or cancelled. The lookup also tells the log which it is.
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
case $conclusion in
  success) ;;
  cancelled | timed_out)
    log "CI for $sha concluded '$conclusion'; not deploying. Nothing else will deploy this SHA: re-run CI (gh run rerun, or the Actions UI)"
    exit 0
    ;;
  *)
    log "CI for $sha concluded '$conclusion'; not deploying"
    exit 0
    ;;
esac

status=$(image_status "$sha")
case $status in
  200) ;;
  404)
    log "CI passed for $sha but $CHARTGEN_IMAGE:$sha is not in the registry yet; waiting"
    exit 0
    ;;
  401 | 403 | token-401 | token-403)
    log "the registry refused to show $CHARTGEN_IMAGE:$sha (answer $status): is the package public? Retrying next tick"
    exit 0
    ;;
  *)
    log "registry lookup for $CHARTGEN_IMAGE:$sha answered $status (rate limit or network); retrying next tick"
    exit 0
    ;;
esac

if ! deploy_image "$sha"; then
  log "retrying next tick"
  exit 1
fi
if ! record_deploy "$sha" || ! write_file "$STATE_FILE" "$sha"; then
  log "deployed $sha but could not write $STATE_FILE"
  exit 1
fi
log "triggered Coolify deploy of $CHARTGEN_IMAGE:$sha"
