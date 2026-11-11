#!/usr/bin/env bash
# Tests chartgen-autodeploy.sh against a stub curl; no network is used.
set -u
here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d) && [ -n "$work" ] || exit 1
trap 'rm -rf "$work"' EXIT
mkdir "$work/bin" || exit 1

# Stub curl. GitHub answers from $GH_HEAD_CODE, $work/head and $CI_JSON, the registry
# from $TOKEN_CODE and $IMAGE_CODE, Coolify from $PATCH_CODE and $COOLIFY_CODE.
# Every call is appended to $work/calls as "METHOD URL [BODY]", never the stdin config.
cat >"$work/bin/curl" <<'STUB' || exit 1
#!/usr/bin/env bash
out=; url=; method=GET; data=
while [ $# -gt 0 ]; do
  case $1 in
    -o) out=$2; shift ;;
    -X) method=$2; shift ;;
    -H | -w | --max-time) shift ;;
    --data) data=$2; shift ;;
    -I) method=HEAD ;;
    -K) cat >/dev/null ;;
    http*) url=$1 ;;
  esac
  shift
done
echo "$method $url $data" >>"$CALLS"
case $url in
  */commits/master)
    echo "{\"sha\":\"$(cat "$HEAD_FILE")\"}" >"$out"
    # Master moves on right after it was read, as when a second push lands mid-run.
    [ -z "${HEAD_MOVES_TO-}" ] || echo "$HEAD_MOVES_TO" >"$HEAD_FILE"
    printf %s "$GH_HEAD_CODE" ;;
  */actions/workflows/ci.yml/runs*) echo "$CI_JSON" >"$out"; printf %s 200 ;;
  */token\?*) echo "$TOKEN_JSON" >"$out"; printf %s "$TOKEN_CODE" ;;
  */manifests/*) printf %s "$IMAGE_CODE" ;;
  */api/v1/applications/*) echo '{}' >"$out"; printf %s "$PATCH_CODE" ;;
  */api/v1/deploy*) echo '{}' >"$out"; printf %s "$COOLIFY_CODE" ;;
  *) echo "unexpected url $url" >&2; exit 9 ;;
esac
STUB
chmod +x "$work/bin/curl" || exit 1

export PATH="$work/bin:$PATH" CALLS="$work/calls" STATE_FILE="$work/state" HEAD_FILE="$work/head"
export COOLIFY_URL=http://coolify.test COOLIFY_API_TOKEN=secret-token COOLIFY_APP_UUID=app-uuid
export GH_HEAD_CODE=200 COOLIFY_CODE=200 PATCH_CODE=200 TOKEN_CODE=200 IMAGE_CODE=200
export TOKEN_JSON='{"token":"anonymous-pull-token"}'
previous="$work/previous-sha"
ci_json() { echo "{\"workflow_runs\":[{\"status\":\"$1\",\"conclusion\":$2}]}"; }

fails=0
fail() { echo "FAIL $*"; fails=$((fails + 1)); }
# calls PATTERN: how many recorded calls match
calls() { grep -c -- "$1" "$CALLS"; }
# run NAME EXPECTED_EXIT EXPECTED_DEPLOYS EXPECTED_STATE(empty = no file) [initial state] [arguments]
# Master's head is bbb unless $HEAD says otherwise. The output of the run is left in $out.
run() {
  local name=$1 want_exit=$2 want_posts=$3 want_state=$4 init=${5-}
  shift 4; [ $# -gt 0 ] && shift
  rm -f "$STATE_FILE" "$CALLS"; touch "$CALLS"
  echo "${HEAD-bbb}" >"$HEAD_FILE"
  [ -n "$init" ] && echo "$init" >"$STATE_FILE"
  local rc posts state=
  out=$("$here/chartgen-autodeploy.sh" "$@" 2>&1); rc=$?
  posts=$(calls '^POST .*/api/v1/deploy')
  [ -f "$STATE_FILE" ] && state=$(cat "$STATE_FILE")
  if [ "$rc" = "$want_exit" ] && [ "$posts" = "$want_posts" ] && [ "$state" = "$want_state" ] &&
     ! grep -q -e secret-token -e anonymous-pull-token <<<"$out"; then
    echo "ok   $name"
  else
    echo "FAIL $name: exit=$rc (want $want_exit) deploys=$posts (want $want_posts) state='$state' (want '$want_state')"
    printf '     %s\n' "${out//$'\n'/$'\n'     }"
    fails=$((fails + 1))
  fi
}

export CI_JSON; CI_JSON=$(ci_json completed '"success"')
run "no state file records head without deploying" 0 0 bbb
[ "$(grep -c . "$CALLS")" = 1 ] || fail "first run made extra calls"

# An empty state file (e.g. a crash mid-write) must re-baseline, not deploy every tick.
rm -f "$CALLS"; : >"$CALLS"; : >"$STATE_FILE"
"$here/chartgen-autodeploy.sh" >/dev/null 2>&1; rc=$?
if [ "$rc" = 0 ] && [ "$(cat "$STATE_FILE")" = bbb ] && [ "$(calls '^POST')" = 0 ]; then
  echo "ok   empty state file counts as first run"
else
  fail "empty state file counts as first run"
fi

run "same SHA exits after one API call" 0 0 bbb bbb
[ "$(grep -c . "$CALLS")" = 1 ] || fail "same-SHA run made extra calls"

CI_JSON=$(ci_json in_progress null)
run "CI pending keeps state, no deploy" 0 0 aaa aaa
[ "$(calls /manifests/)" = 0 ] || fail "CI pending asked the registry"
CI_JSON='{"workflow_runs":[]}'
run "no CI run yet keeps state, no deploy" 0 0 aaa aaa
CI_JSON=$(ci_json completed '"failure"')
run "CI failed keeps state, no deploy" 0 0 aaa aaa
[ "$(calls /api/v1/)" = 0 ] || fail "CI failed reached Coolify"

for c in cancelled timed_out; do
  CI_JSON=$(ci_json completed "\"$c\"")
  run "CI $c keeps state, no deploy" 0 0 aaa aaa
  grep -q 'gh run rerun' <<<"$out" || fail "$c log lacks re-run hint"
done
CI_JSON=$(ci_json completed '"failure"')
run "CI failed says nothing of a re-run" 0 0 aaa aaa
grep -q 'gh run rerun' <<<"$out" && fail "test failure log has re-run hint"

CI_JSON=$(ci_json completed '"success"')
rm -f "$previous"
run "CI success and image there: tag set, deployed once, state written" 0 1 bbb aaa
[ "$(calls '^PATCH http://coolify.test/api/v1/applications/app-uuid {"docker_registry_image_name":"ghcr.io/5queezer/chartgen","docker_registry_image_tag":"bbb"}$')" = 1 ] ||
  fail "the image tag was not set to the SHA: $(grep PATCH "$CALLS")"
[ "$(calls '^HEAD https://ghcr.io/v2/5queezer/chartgen/manifests/bbb')" = 1 ] || fail "the registry was not asked for the SHA's image"
[ "$(grep -n -e '^PATCH' -e '^POST' "$CALLS" | cut -d' ' -f1 | tr '\n' ' ')" = "5:PATCH 6:POST " ] || fail "tag and deploy out of order: $(cat "$CALLS")"
[ "$(cat "$previous" 2>/dev/null)" = aaa ] || fail "the SHA deployed before was not recorded"

# The image is built by the CI run that passed, so it may be a moment behind it.
IMAGE_CODE=404 run "image not published yet keeps state, no deploy" 0 0 aaa aaa
grep -q 'not in the registry yet' <<<"$out" || fail "missing image is not named in the log"
[ "$(calls /api/v1/)" = 0 ] || fail "missing image reached Coolify"
# Whatever else the registry says is "try next tick": nothing is deployed on a guess.
IMAGE_CODE=429 run "registry rate limit keeps state, no deploy" 0 0 aaa aaa
IMAGE_CODE=500 run "registry error keeps state, no deploy" 0 0 aaa aaa
IMAGE_CODE=000 run "registry unreachable keeps state, no deploy" 0 0 aaa aaa
IMAGE_CODE=401 run "registry refuses the image keeps state, no deploy" 0 0 aaa aaa
grep -q 'is the package public' <<<"$out" || fail "a refused image does not ask about visibility"
TOKEN_CODE=429 run "registry token rate limit keeps state, no deploy" 0 0 aaa aaa
[ "$(calls /manifests/)" = 0 ] || fail "asked for the image without a token"
TOKEN_CODE=403 run "registry token refused keeps state, no deploy" 0 0 aaa aaa
grep -q 'is the package public' <<<"$out" || fail "a refused token does not ask about visibility"
TOKEN_JSON='{}' run "registry token missing keeps state, no deploy" 0 0 aaa aaa
TOKEN_JSON='not json' run "registry token unreadable keeps state, no deploy" 0 0 aaa aaa
[ "$(calls /api/v1/)" = 0 ] || fail "an unreadable token reached Coolify"

rm -f "$previous"
PATCH_CODE=500 run "Coolify refuses the tag: no deploy, state kept" 1 0 aaa aaa
[ ! -e "$previous" ] || fail "a failed tag change recorded a previous SHA"
COOLIFY_CODE=502 run "Coolify 5xx on deploy keeps state" 1 1 aaa aaa
[ ! -e "$previous" ] || fail "a failed deploy recorded a previous SHA"
# The next tick does both again: setting the same tag twice changes nothing.
run "after a failed deploy the next tick sets the tag and deploys" 0 1 bbb aaa
[ "$(calls '^PATCH')" = 1 ] || fail "the retry did not set the tag again"
GH_HEAD_CODE=403 run "GitHub 403 exits 0 without deploying" 0 0 aaa aaa

# Master moves while a run is under way: the run deploys the SHA it started with, which
# passed CI and has its image, and the next tick sees the newer head.
HEAD_MOVES_TO=ccc run "head moves mid-run: the SHA read first is deployed" 0 1 bbb aaa
[ "$(calls '"docker_registry_image_tag":"bbb"')" = 1 ] || fail "deployed another SHA than the one read"
[ "$(cat "$HEAD_FILE")" = ccc ] || fail "the stub did not move master"
IMAGE_CODE=404 HEAD=ccc run "the next tick waits for the newer head's image" 0 0 bbb bbb

# By hand. Neither touches the timer's state, so the timer does not undo a rollback.
echo aaa >"$previous"
run "rollback deploys the SHA from before the last deploy" 0 1 bbb bbb rollback
[ "$(calls '"docker_registry_image_tag":"aaa"')" = 1 ] || fail "rollback set another tag"
[ "$(calls api.github.com)" = 0 ] || fail "rollback asked GitHub"
run "the tick after a rollback deploys nothing" 0 0 bbb bbb
run "deploy SHA deploys that image" 0 1 bbb bbb deploy 0123abc
[ "$(calls '"docker_registry_image_tag":"0123abc"')" = 1 ] || fail "deploy set another tag"
IMAGE_CODE=404 run "deploy of a SHA with no image fails before Coolify" 1 0 bbb bbb deploy 0123abc
[ "$(calls /api/v1/)" = 0 ] || fail "a missing image reached Coolify"
PATCH_CODE=500 run "a failed rollback exits 1" 1 0 bbb bbb rollback
rm -f "$previous"
run "rollback with nothing recorded fails and says how" 1 0 bbb bbb rollback
grep -q 'deploy SHA' <<<"$out" || fail "rollback without a previous SHA gives no way out"
run "deploy without a SHA is a usage error" 2 0 bbb bbb deploy
run "an unknown command is a usage error" 2 0 bbb bbb sideways

if [ "$fails" = 0 ]; then echo "all passed"; else echo "$fails failed"; exit 1; fi
