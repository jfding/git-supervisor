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
mkdir -p "$TMP/git_repos/my.api"                         # multi-dot repo name
mkdir -p "$TMP/copies/my.api.prod.v1.to-be-removed"      # stale, must attribute to my.api (not my)
mkdir -p "$TMP/copies/orphan.test.to-be-removed"         # stale, no matching repo → repo "-"

# ---- dry-run (APPLY unset → default 0) ----
out=$(DIR_BASE="$TMP" HOST_ID="h1" bash "$PROBE")
contains "dry-run lists webapp stale" "$out" $'h1\twebapp\twebapp.prod.v10.0.to-be-removed\t'
contains "dry-run lists api stale"    "$out" $'h1\tapi-service\tapi-service.dev.to-be-removed\t'
contains "dry-run marks would-remove" "$out" "would-remove"
contains "dry-run attributes multi-dot repo" "$out" $'h1\tmy.api\tmy.api.prod.v1.to-be-removed\t'
contains "dry-run emits '-' for orphan stale" "$out" $'h1\t-\torphan.test.to-be-removed\t'
check    "dry-run does NOT list live copy" "$(echo "$out" | grep -c 'webapp.main')" "0"
check    "dry-run deleted nothing (stale dir still present)" \
         "$([[ -d "$TMP/copies/webapp.prod.v10.0.to-be-removed" ]] && echo yes || echo no)" "yes"

# ---- apply ----
out=$(DIR_BASE="$TMP" HOST_ID="h1" APPLY=1 bash "$PROBE")
ap_rc=$?
contains "apply marks webapp removed" "$out" $'h1\twebapp\twebapp.prod.v10.0.to-be-removed\t'
contains "apply outcome removed present" "$out" "removed"
check    "apply deleted webapp stale dir" \
         "$([[ -d "$TMP/copies/webapp.prod.v10.0.to-be-removed" ]] && echo yes || echo no)" "no"
check    "apply deleted api stale dir" \
         "$([[ -d "$TMP/copies/api-service.dev.to-be-removed" ]] && echo yes || echo no)" "no"
check    "apply left live copy untouched" \
         "$([[ -d "$TMP/copies/webapp.main" ]] && echo yes || echo no)" "yes"
check    "apply exit code 0 on full success" "$ap_rc" "0"

# ---- safe-rm refusal: a symlink pointing outside DIR_COPIES must not be followed/deleted ----
OUTSIDE=$(mktemp -d)
trap 'rm -rf "$TMP" "$OUTSIDE"' EXIT
mkdir -p "$OUTSIDE/precious"
ln -s "$OUTSIDE/precious" "$TMP/copies/evil.to-be-removed"
out=$(DIR_BASE="$TMP" HOST_ID="h1" APPLY=1 bash "$PROBE")
ev_rc=$?
contains "refuses to delete outside copies tree" "$out" "failed"
check    "outside target NOT deleted" \
         "$([[ -d "$OUTSIDE/precious" ]] && echo yes || echo no)" "yes"
check    "apply exit code non-zero when a deletion fails" "$([[ $ev_rc -ne 0 ]] && echo nz || echo z)" "nz"
rm -rf "$OUTSIDE"

echo "---- $fails failure(s) ----"
exit $((fails > 0))
