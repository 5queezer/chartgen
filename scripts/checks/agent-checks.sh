#!/usr/bin/env bash
# Checks that replace notes in CLAUDE.md ("Agent notes"): what an agent could miss in prose
# fails here instead. Run from the repository root; `scripts/checks/test.sh` proves each fires.
#
#   agent-checks.sh                         both checks, over the repository
#   agent-checks.sh todo-links [file...]    TODO, FIXME and HACK comments name an issue
#   agent-checks.sh press-points [file...]  Playwright specs press at measured points
set -u

# A TODO, FIXME or HACK must say where its work is tracked: an issue (#123) or a link.
todo_links() {
  local out
  if [ $# -gt 0 ]; then
    out=$(grep -nIHE '\b(TODO|FIXME|HACK)\b' "$@" | grep -vE '#[0-9]+|https?://')
  else
    out=$(git grep -nIE '\b(TODO|FIXME|HACK)\b' -- . ':!scripts/checks/fixtures' | grep -vE '#[0-9]+|https?://')
  fi
  [ -z "$out" ] && return 0
  printf '%s\n' "$out"
  echo "agent-checks: a TODO, FIXME or HACK comment needs an issue (#123) or a link to where it is tracked." >&2
  return 1
}

# A press on the chart takes its point from chart.getSize(...) or a bounding box, never from
# literal pixels: a layout change moves a legend row or pane separator under a fixed point.
# Literal coordinates are refused in mouse.click/dblclick and in a mouse.move right before a
# mouse.down. A press that has to keep one says why with `e2e-fixed-point:` in a comment on
# its line or the line before.
press_points() {
  local files out
  if [ $# -gt 0 ]; then files="$*"; else files=$(git ls-files -- web/tests/e2e | grep -E '\.(ts|js)$'); fi
  [ -z "$files" ] && return 0
  # shellcheck disable=SC2086
  out=$(awk '
    function lit(s) { return s ~ /mouse\.(move|click|dblclick)\([[:space:]]*-?[0-9.]+[[:space:]]*,[[:space:]]*-?[0-9.]+/ }
    FNR == 1 { held = ""; skip = 0 }
    /e2e-fixed-point:/ { held = ""; skip = FNR + 1; next }
    FNR == skip { held = ""; next }
    /mouse\.(click|dblclick)\(/ && lit($0) { print FILENAME ":" FNR ": " $0; held = ""; next }
    /mouse\.move\(/ { held = lit($0) ? FILENAME ":" FNR ": " $0 : ""; next }
    /mouse\.down\(/ { if (held != "") print held; held = ""; next }
    /[^[:space:]]/ { held = "" }
  ' $files)
  [ -z "$out" ] && return 0
  printf '%s\n' "$out"
  echo "agent-checks: press at a point from chart.getSize(...) or a bounding box, not at literal pixels (or say why with e2e-fixed-point:)." >&2
  return 1
}

case "${1:-all}" in
  todo-links) shift; todo_links "$@" ;;
  press-points) shift; press_points "$@" ;;
  all) status=0; todo_links || status=1; press_points || status=1; exit $status ;;
  *) echo "usage: $0 [todo-links|press-points] [path...]" >&2; exit 2 ;;
esac
