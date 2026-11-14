#!/usr/bin/env bash
# Proves each check in agent-checks.sh fires on its bad fixture and passes its good one.
set -u
cd "$(git rev-parse --show-toplevel)" || exit 2
check=scripts/checks/agent-checks.sh
fx=scripts/checks/fixtures
fail=0
expect() { # expect <0|1> <what> <command...>
  local want=$1 what=$2; shift 2
  local out got
  out=$("$@" 2>&1); got=$?
  if [ "$got" != "$want" ]; then
    echo "FAIL: $what exited $got, expected $want"; printf '%s\n' "$out"; fail=1
  else
    echo "ok: $what"
  fi
}
expect_lines() { # expect_lines <n> <what> <command...>: the check names n lines
  local want=$1 what=$2; shift 2
  local got
  got=$("$@" 2>/dev/null | grep -c .)
  if [ "$got" = "$want" ]; then echo "ok: $what"; else echo "FAIL: $what: $got lines, expected $want"; fail=1; fi
}
expect 1 "todo-links refuses unlinked markers" "$check" todo-links "$fx/todo.bad.txt"
expect_lines 3 "todo-links finds all three" "$check" todo-links "$fx/todo.bad.txt"
expect 0 "todo-links accepts linked markers" "$check" todo-links "$fx/todo.good.txt"
expect 1 "press-points refuses literal presses" "$check" press-points "$fx/press-points.bad.spec.ts"
expect_lines 3 "press-points finds all three" "$check" press-points "$fx/press-points.bad.spec.ts"
expect 0 "press-points accepts measured presses" "$check" press-points "$fx/press-points.good.spec.ts"
expect 0 "the repository passes" "$check"
exit $fail
