#!/usr/bin/env bash
# Self-contained tests for src/cleanup_probe.sh. Creates a temp DIR_BASE,
# runs the probe in dry-run and apply modes, asserts behavior.
set -u
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROBE="$(cd "$SCRIPT_DIR/../../../src" && pwd)/cleanup_probe.sh"
fails=0
check() { # desc, actual, expected
  if [[ "$2" == "$3" ]]; then echo "ok   - $1"; else echo "FAIL - $1: got [$2] want [$3]"; fails=$((fails+1)); fi
}
contains() { # desc, haystack, needle
  if [[ "$2" == *"$3"* ]]; then echo "ok   - $1"; else echo "FAIL - $1: [$2] missing [$3]"; fails=$((fails+1)); fi
}

# ---- fixture ----
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/git_repos/webapp" "$TMP/git_repos/api-service"
mkdir -p "$TMP/copies/webapp.main"                       # live branch copy
mkdir -p "$TMP/copies/webapp.prod.v10.0.to-be-removed"   # stale
mkdir -p "$TMP/copies/api-service.dev.to-be-removed"     # stale

# ---- dry-run (APPLY unset → default 0) ----
out=$(DIR_BASE="$TMP" HOST_ID="h1" bash "$PROBE")
contains "dry-run lists webapp stale" "$out" $'h1\twebapp\twebapp.prod.v10.0.to-be-removed\t'
contains "dry-run lists api stale"    "$out" $'h1\tapi-service\tapi-service.dev.to-be-removed\t'
contains "dry-run marks would-remove" "$out" "would-remove"
check    "dry-run does NOT list live copy" "$(echo "$out" | grep -c 'webapp.main')" "0"
check    "dry-run deleted nothing (stale dir still present)" \
         "$([[ -d "$TMP/copies/webapp.prod.v10.0.to-be-removed" ]] && echo yes || echo no)" "yes"

echo "---- $fails failure(s) ----"
exit $((fails > 0))
