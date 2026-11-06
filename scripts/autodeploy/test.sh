#!/usr/bin/env bash
# Tests chartgen-autodeploy.sh against a stub curl; no network is used.
set -u
here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d) && [ -n "$work" ] || exit 1
trap 'rm -rf "$work"' EXIT
mkdir "$work/bin" || exit 1

# Stub curl: GitHub URLs answer from $GH_HEAD_CODE/$CI_JSON, the Coolify POST from $COOLIFY_CODE.
# Every call is appended to $work/calls (URL only, never the stdin config).
cat >"$work/bin/curl" <<'STUB' || exit 1
#!/usr/bin/env bash
out=; url=; method=GET
while [ $# -gt 0 ]; do
  case $1 in
    -o) out=$2; shift ;;
    -X) method=$2; shift ;;
    -K) cat >/dev/null ;;
    http*) url=$1 ;;
  esac
  shift
done
echo "$method $url" >>"$CALLS"
case $url in
  */commits/master) echo "{\"sha\":\"$HEAD_SHA\"}" >"$out"; printf %s "$GH_HEAD_CODE" ;;
  */actions/workflows/ci.yml/runs*) echo "$CI_JSON" >"$out"; printf %s 200 ;;
  */api/v1/deploy*) echo '{}' >"$out"; printf %s "$COOLIFY_CODE" ;;
  *) echo "unexpected url $url" >&2; exit 9 ;;
esac
STUB
chmod +x "$work/bin/curl" || exit 1

export PATH="$work/bin:$PATH" CALLS="$work/calls" STATE_FILE="$work/state"
export COOLIFY_URL=http://coolify.test COOLIFY_API_TOKEN=secret-token COOLIFY_APP_UUID=app-uuid
export HEAD_SHA=bbb GH_HEAD_CODE=200 COOLIFY_CODE=200
ci_json() { echo "{\"workflow_runs\":[{\"status\":\"$1\",\"conclusion\":$2}]}"; }

fails=0
# run NAME EXPECTED_EXIT EXPECTED_POSTS EXPECTED_STATE(empty = no file) [initial state]
run() {
  local name=$1 want_exit=$2 want_posts=$3 want_state=$4 init=${5-}
  rm -f "$STATE_FILE" "$CALLS"; touch "$CALLS"
  [ -n "$init" ] && echo "$init" >"$STATE_FILE"
  local out rc posts state=
  out=$("$here/chartgen-autodeploy.sh" 2>&1); rc=$?
  posts=$(grep -c '^POST' "$CALLS")
  [ -f "$STATE_FILE" ] && state=$(cat "$STATE_FILE")
  if [ "$rc" = "$want_exit" ] && [ "$posts" = "$want_posts" ] && [ "$state" = "$want_state" ] &&
     ! grep -q secret-token <<<"$out"; then
    echo "ok   $name"
  else
    echo "FAIL $name: exit=$rc (want $want_exit) posts=$posts (want $want_posts) state='$state' (want '$want_state')"
    printf '     %s\n' "${out//$'\n'/$'\n'     }"
    fails=$((fails + 1))
  fi
}

export CI_JSON; CI_JSON=$(ci_json completed '"success"')
run "no state file records head without deploying" 0 0 bbb
[ "$(grep -c . "$CALLS")" = 1 ] || { echo "FAIL first run made extra calls"; fails=$((fails + 1)); }

# An empty state file (e.g. a crash mid-write) must re-baseline, not deploy every tick.
rm -f "$CALLS"; : >"$CALLS"; : >"$STATE_FILE"
"$here/chartgen-autodeploy.sh" >/dev/null 2>&1; rc=$?
if [ "$rc" = 0 ] && [ "$(cat "$STATE_FILE")" = bbb ] && [ "$(grep -c '^POST' "$CALLS")" = 0 ]; then
  echo "ok   empty state file counts as first run"
else
  echo "FAIL empty state file counts as first run"; fails=$((fails + 1))
fi

run "same SHA exits after one API call" 0 0 bbb bbb
[ "$(grep -c . "$CALLS")" = 1 ] || { echo "FAIL same-SHA run made extra calls"; fails=$((fails + 1)); }

CI_JSON=$(ci_json in_progress null)
run "CI pending keeps state, no deploy" 0 0 aaa aaa
CI_JSON='{"workflow_runs":[]}'
run "no CI run yet keeps state, no deploy" 0 0 aaa aaa
CI_JSON=$(ci_json completed '"failure"')
run "CI failed keeps state, no deploy" 0 0 aaa aaa

for c in cancelled timed_out; do
  CI_JSON=$(ci_json completed "\"$c\"")
  run "CI $c keeps state, no deploy" 0 0 aaa aaa
  grep -q 'gh run rerun' <<<"$("$here/chartgen-autodeploy.sh" 2>&1)" || { echo "FAIL $c log lacks re-run hint"; fails=$((fails + 1)); }
done
CI_JSON=$(ci_json completed '"failure"')
[ "$(echo aaa >"$STATE_FILE"; "$here/chartgen-autodeploy.sh" 2>&1 | grep -c 'gh run rerun')" = 0 ] || { echo "FAIL test failure log has re-run hint"; fails=$((fails + 1)); }

CI_JSON=$(ci_json completed '"success"')
run "CI success posts once and writes state" 0 1 bbb aaa
COOLIFY_CODE=502 run "Coolify 5xx keeps state" 1 1 aaa aaa
COOLIFY_CODE=200
GH_HEAD_CODE=403 run "GitHub 403 exits 0 without deploying" 0 0 aaa aaa
GH_HEAD_CODE=200

if [ "$fails" = 0 ]; then echo "all passed"; else echo "$fails failed"; exit 1; fi
