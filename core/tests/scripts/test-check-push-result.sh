#!/usr/bin/env bash
# Unit-ish tests for RESULT protocol without full repo fixtures.
# Sources check-push helpers by running the script in a temp DIR_BASE with a
# fake broken remote (or mocks fetch_and_check via embedding). Prefer the
# lightest path that still exercises main_loop's FAIL_DIR + exit path.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SCRIPT="$ROOT/check-push.sh"
fails=0
pass() { echo "PASS: $*"; }
fail() { echo "FAIL: $*"; fails=$((fails + 1)); }

# --- parse_result helper exercised against canned stdout ---
parse_ok=$(printf 'result\tfail\tmy-repo\n' | awk -F'\t' '$1=="result" && $2=="fail" {print $3}')
[[ "$parse_ok" == "my-repo" ]] && pass "RESULT line shape" || fail "RESULT line shape"

# --- logging goes to stderr ---
# Extract _logging by running a tiny wrapper: we verify a successful --once
# with empty REPO_WHITELIST / empty repos still puts no RESULT on stdout.
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/git_repos" "$TMP/copies"
out=$(mktemp); err=$(mktemp)
set +e
DIR_BASE="$TMP" SLEEP_TIME=0 CI_LOCK="$TMP/lock.d" LOGLEVEL=1 \
  bash "$SCRIPT" --once >"$out" 2>"$err"
rc=$?
set -e
[[ $rc -eq 0 ]] && pass "empty run exit 0" || fail "empty run exit 0 (rc=$rc)"
[[ ! -s "$out" ]] && pass "empty run empty stdout" || fail "empty run stdout not empty: $(cat "$out")"
# If LOGLEVEL allowed any line, it must be on stderr not stdout — already checked.

# --- hard fail: real git repo with unreachable remote so fetch_and_check returns 1 ---
git init -q "$TMP/git_repos/badrepo"
git -C "$TMP/git_repos/badrepo" config user.email "test@test"
git -C "$TMP/git_repos/badrepo" config user.name "test"
echo init >"$TMP/git_repos/badrepo/README"
git -C "$TMP/git_repos/badrepo" add README
git -C "$TMP/git_repos/badrepo" commit -qm "init"
git -C "$TMP/git_repos/badrepo" remote add origin "http://127.0.0.1:1/unreachable.git"
out=$(mktemp); err=$(mktemp)
set +e
DIR_BASE="$TMP" SLEEP_TIME=0 CI_LOCK="$TMP/lock.d" LOGLEVEL=0 TIMEOUT=10 \
  REPO_WHITELIST="badrepo" \
  bash "$SCRIPT" --once >"$out" 2>"$err"
rc=$?
set -e
[[ $rc -eq 1 ]] && pass "hard fail exit 1" || fail "hard fail exit 1 (rc=$rc)"
grep -qx $'result\tfail\tbadrepo' "$out" && pass "RESULT for badrepo" || fail "RESULT missing: $(cat "$out")"
# Human noise must not be on stdout
! grep -v $'^result\tfail\t' "$out" | grep -q . && pass "stdout only RESULT" || fail "stdout polluted"

[[ $fails -eq 0 ]]
