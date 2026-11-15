#!/usr/bin/env bash
# Pull-based deploy: poll GitHub for master's head, and when CI passed for a
# new SHA and its image is in the registry, point the Coolify application at
# that image and trigger the deploy API. One run per systemd timer tick.
#
#   chartgen-autodeploy.sh               one tick
#   chartgen-autodeploy.sh deploy SHA    deploy the image of SHA now
#   chartgen-autodeploy.sh rollback      deploy the image that ran before the one that runs now
#   chartgen-autodeploy.sh running SHA   record that SHA is what runs now; deploys nothing
#
# A deploy Coolify accepted is "triggered". The ticks after it follow that deployment
# in Coolify until it finished, and only then is its image recorded as running. One
# that failed is deployed again, at most DEPLOY_ATTEMPTS times in all, a tick apart.
#
# See README.md, ADR-0037, ADR-0060 and ADR-0075.
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
# What this script triggered last, by the timer or by hand, or was told runs: triggered,
# not ran. The watchdog compares it with the image production runs.
DEPLOYED_FILE=$(dirname "$STATE_FILE")/deployed-sha
# The image Coolify last finished deploying, or that the script was told runs, and the
# one that ran before it: where a rollback goes.
RUNNING_FILE=$(dirname "$STATE_FILE")/running-sha
PREVIOUS_FILE=$(dirname "$STATE_FILE")/previous-sha
# A deploy being followed: "SHA DEPLOYMENT ATTEMPT SINCE", SINCE the epoch second of its
# first attempt. An attempt is written here before it is made.
PENDING_FILE=$(dirname "$STATE_FILE")/pending
DEPLOY_ATTEMPTS=${DEPLOY_ATTEMPTS:-3}
# Followed for this long at most: past it, a deploy still queued or running is left to
# the operator.
DEPLOY_WAIT_SECS=${DEPLOY_WAIT_SECS:-2700}
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
# Returns 0 when Coolify accepted both calls, which is "triggered", not "healthy", and
# sets deployment to the deployment Coolify named. An answer that names none is a
# refusal: nothing was queued.
deploy_image() {
  deployment=
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
  deployment=$(jq -r '.deployments[0].deployment_uuid // empty' "$body" 2>/dev/null)
  if [ -z "$deployment" ]; then
    # Coolify names one whenever it queued a deployment, also one already queued for
    # this image. Without it nothing was queued: a full queue, or a refusal.
    log "Coolify named no deployment for $1 ($(jq -r '.deployments[0].message // .message // "no message"' "$body" 2>/dev/null)); not deployed"
    return 1
  fi
}

# record_deploy SHA ATTEMPT: SHA is what this script triggered last, and the deployment
# Coolify named for it is the one the next ticks follow.
record_deploy() {
  write_file "$DEPLOYED_FILE" "$1" &&
    write_file "$PENDING_FILE" "$1 ${deployment:--} $2 ${3:-$(date +%s)}"
}

# record_ran SHA: Coolify finished deploying SHA, so it runs now, and what ran until now
# is what a rollback goes back to. The timer's mark is not used for that: it is a head
# of master the timer acted on, and on a first run acting on it means recording it
# without deploying. With nothing known to have run before, there is no previous
# image, and a rollback says so.
record_ran() {
  local before=
  [ -s "$RUNNING_FILE" ] && before=$(cat "$RUNNING_FILE")
  if [ -z "$before" ]; then
    # The first image known to run. A previous-sha from before is an older script's
    # "triggered before", which may never have run: a rollback must not go there.
    rm -f "$PREVIOUS_FILE" || return 1
  elif [ "$before" != "$1" ]; then
    write_file "$PREVIOUS_FILE" "$before" || return 1
  fi
  write_file "$RUNNING_FILE" "$1"
}

# pending_status: the deploy being followed, read into p_sha, p_uuid, p_attempt and
# p_since, and where it stands in Coolify into p_status (empty when that could not be
# read) and p_code. Returns 1 when no deploy is followed. Names of their own: a caller's
# `local sha` would otherwise be the variable this sets.
pending_status() {
  p_status='' p_code=''
  [ -s "$PENDING_FILE" ] || return 1
  read -r p_sha p_uuid p_attempt p_since <"$PENDING_FILE"
  # A line cut short, by hand or by a disk that filled, still ends in a decision.
  [[ ${p_uuid-} =~ ^[A-Za-z0-9_-]+$ ]] || p_uuid=-
  [[ ${p_attempt-} =~ ^[0-9]+$ ]] || p_attempt=1
  [[ ${p_since-} =~ ^[0-9]+$ ]] || p_since=$(date +%s)
  if [ "$p_uuid" != - ]; then
    p_code=$(coolify GET "/deployments/$p_uuid")
    case $p_code in
      2??) p_status=$(jq -r '.status // empty' "$body" 2>/dev/null) ;;
    esac
  fi
  return 0
}

# finished: the deploy just read is finished; record it as running, follow it no more.
# Recorded first: a write that fails leaves it followed, and the next tick records it.
finished() {
  record_ran "$p_sha" || { log "Coolify finished deploying $p_sha, but it could not be recorded as running; trying again next tick"; return 1; }
  rm -f "$PENDING_FILE"
  log "Coolify finished deploying $CHARTGEN_IMAGE:$p_sha"
}

# before_hand: what a command by hand does first with the deploy being followed. One
# that finished is recorded, so that what ran before is known to a rollback; one that
# failed or was cancelled is no longer followed: the command replaces it. One still
# queued or running, or that cannot be read, is not replaced: it can still finish, and
# then it runs. Returns 1 then, and the command deploys nothing.
before_hand() {
  pending_status || return 0
  case $p_status in
    finished) finished ;;
    failed | cancelled-by-user)
      rm -f "$PENDING_FILE"
      log "the deploy of $p_sha ($p_status in Coolify) is replaced by this one"
      ;;
    *)
      log "the deploy of $p_sha is ${p_status:-not readable} in Coolify (deployment $p_uuid): it can still finish. Wait for it to end, which the timer follows, or cancel it in Coolify; then run this again. Nothing was deployed"
      return 1
      ;;
  esac
}

# follow_deploy: where the deploy being followed stands in Coolify, and what to do
# about it. Returns 0 when none is followed any more and the tick may go on, 1 when this
# tick has done its part. Sets gave_up when it stopped following one that had not
# finished.
follow_deploy() {
  local now
  pending_status || return 0
  now=$(date +%s)
  if [ "$p_uuid" = - ]; then
    rm -f "$PENDING_FILE"
    gave_up=1
    log "ERROR: the deploy of $CHARTGEN_IMAGE:$p_sha cannot be followed: its record names no Coolify deployment. Look at Coolify's deployments; then: running SHA, or deploy $p_sha"
    return 1
  fi
  case $p_status in
    finished)
      finished
      return
      ;;
    failed)
      if [ "$p_attempt" -ge "$DEPLOY_ATTEMPTS" ]; then
        rm -f "$PENDING_FILE"
        gave_up=1
        log "ERROR: the deploy of $CHARTGEN_IMAGE:$p_sha failed in Coolify $p_attempt times (last deployment $p_uuid); not deploying it again. Production runs $(cat "$RUNNING_FILE" 2>/dev/null || echo 'what it ran before'). Look at the deployment in Coolify; when the cause is gone: deploy $p_sha"
        return 1
      fi
      # The attempt is written before it is made: one that cannot be recorded is not
      # made, and every attempt made counts, refused or not.
      p_attempt=$((p_attempt + 1))
      if ! write_file "$PENDING_FILE" "$p_sha $p_uuid $p_attempt $p_since"; then
        gave_up=1
        log "ERROR: the deploy of $p_sha failed in Coolify (deployment $p_uuid), and attempt $p_attempt could not be recorded in $(dirname "$STATE_FILE"); not deploying it again"
        return 1
      fi
      if ! deploy_image "$p_sha"; then
        log "the deploy of $p_sha failed in Coolify (deployment $p_uuid), and Coolify refused it again; attempt $p_attempt of $DEPLOY_ATTEMPTS, trying again next tick"
      elif ! record_deploy "$p_sha" "$p_attempt" "$p_since"; then
        gave_up=1
        log "ERROR: deployed $p_sha again (deployment $deployment), but could not record it; the next tick follows the failed one and counts on from attempt $p_attempt"
      else
        log "the deploy of $p_sha failed in Coolify (deployment $p_uuid); deploying it again, attempt $p_attempt of $DEPLOY_ATTEMPTS"
      fi
      return 1
      ;;
    cancelled-by-user)
      rm -f "$PENDING_FILE"
      log "the deploy of $p_sha was cancelled in Coolify; not deploying it again. To put it out: deploy $p_sha"
      return 1
      ;;
  esac
  if [ $((now - p_since)) -gt "$DEPLOY_WAIT_SECS" ]; then
    rm -f "$PENDING_FILE"
    gave_up=1
    log "ERROR: the deploy of $CHARTGEN_IMAGE:$p_sha is not finished after $(((now - p_since) / 60)) min (deployment $p_uuid, ${p_status:-status unknown}); no longer following it. Look at it in Coolify"
    return 1
  fi
  case $p_status in
    queued | in_progress) log "the deploy of $p_sha is $p_status in Coolify; waiting" ;;
    *) log "could not read the deploy of $p_sha from Coolify (deployment $p_uuid, HTTP ${p_code:-none}${p_status:+, status $p_status}); asking again next tick" ;;
  esac
  return 1
}

# A command by hand is the operator's word on what runs, so the timer must not put
# master's present head out over it: that head may be the very commit that was just
# rolled back. The timer's mark is moved to master's head as it is now, without
# deploying it. The next commit on master deploys as usual. If GitHub cannot be asked
# now, the next tick that can ask does the same.
# Sets head_now to that head, or to nothing if GitHub could not be asked.
leave_head_alone() {
  local code
  code=$(gh_get "$API/commits/master")
  head_now=
  [ "$code" = 200 ] && head_now=$(jq -r '.sha // empty' "$body" 2>/dev/null)
  if [ -n "$head_now" ]; then
    rm -f "$HAND_FILE"
    write_file "$STATE_FILE" "$head_now"
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
  before_hand || exit 1
  deploy_image "$sha" || exit 1
  if ! record_deploy "$sha" 1 || ! leave_head_alone; then
    log "deployed $sha by hand but could not record it in $(dirname "$STATE_FILE"); the timer may deploy master's head over it"
    exit 1
  fi
  log "triggered Coolify deploy of $CHARTGEN_IMAGE:$sha by hand; the timer's ticks follow it until Coolify finished it"
  if [ -z "$head_now" ]; then
    log "GitHub could not be asked where master is: the first tick that can ask will leave that head alone and name it. The next commit after it deploys as usual"
  elif [ "$head_now" != "$sha" ]; then
    log "master's head $head_now is not deployed by the timer; to put it out: deploy $head_now. The next commit on master deploys as usual"
  fi
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
    before_hand || exit 1
    if [ ! -s "$RUNNING_FILE" ]; then
      log "this script does not know what runs now, so it knows nothing to roll back to. Tell it with: running SHA (the commit of the image that runs), or put an image out with: deploy SHA"
      exit 1
    fi
    if [ ! -s "$PREVIOUS_FILE" ]; then
      log "this script knows no image that ran before the one that runs now, so there is nothing to roll back to. Read the SHA that should run from Coolify or the commit log and use: deploy SHA"
      exit 1
    fi
    log "rolling back to $(cat "$PREVIOUS_FILE"), the image that ran before $(cat "$RUNNING_FILE" 2>/dev/null || echo the one that runs now)"
    manual "$(cat "$PREVIOUS_FILE")"
    exit 0
    ;;
  running)
    if ! [[ ${2-} =~ ^[0-9a-f]{40}$ ]]; then
      echo "usage: $0 running SHA (the full commit SHA, 40 hex characters)" >&2
      exit 2
    fi
    take_turn hand
    # The operator's word: what runs now, and no deploy is followed any more.
    # Through record_ran, so that what ran before it is where a rollback goes.
    if ! write_file "$DEPLOYED_FILE" "$2" || ! record_ran "$2" || ! rm -f "$PENDING_FILE"; then
      log "could not record $2 in $(dirname "$STATE_FILE")"
      exit 1
    fi
    log "recorded $2 as what runs now; nothing was deployed"
    exit 0
    ;;
  *)
    echo "usage: $0 [deploy SHA | rollback | running SHA]" >&2
    exit 2
    ;;
esac

take_turn tick
# A deploy triggered before is followed first; while it is, nothing else is deployed.
gave_up=
if ! follow_deploy; then
  [ -z "$gave_up" ] || exit 1
  exit 0
fi
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
if ! record_deploy "$sha" 1 || ! write_file "$STATE_FILE" "$sha"; then
  log "deployed $sha but could not write $STATE_FILE"
  exit 1
fi
log "triggered Coolify deploy of $CHARTGEN_IMAGE:$sha; the next ticks follow it until Coolify finished it"
