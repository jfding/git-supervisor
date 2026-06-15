#!/bin/bash

# Multi-run integration tests for check-push.sh
#
# Assumes the first run (test-check-push.sh) has already completed.
# This script mutates the fake remotes, re-runs check-push.sh --once,
# and verifies that dynamic changes are picked up correctly:
#
#   1. New commits pushed to remote  → copy dir updated, .git-rev changes
#   2. New tag pushed to remote      → new release copy, prod.latest updated
#   3. Branch deleted from remote    → copy marked .to-be-removed
#   4. Unchanged branches            → idempotent (no unnecessary updates)

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEV_DIR="$(cd "$(dirname "$SCRIPT_DIR")/work.test" && pwd)"
CHECK_PUSH_SCRIPT="$DEV_DIR/../../check-push.sh"

export DIR_BASE="$DEV_DIR"
export CI_LOCK="$DEV_DIR/.ci-lock.d"
export VERB=2
export TIMEOUT=30
export SLEEP_TIME=""

FAKE_DOCKER_DIR="$DIR_BASE/.fake-bin"

echo ""
echo "=== Multi-run integration tests ==="
echo ""

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
_fail() { echo "FAIL: $*" >&2; exit 1; }
_ok()   { echo "  OK: $*"; }

# Record .git-rev for a copy dir (empty string if missing)
_read_git_rev() {
  cat "$DIR_BASE/copies/$1/.git-rev" 2>/dev/null || true
}

# Push a new commit to a fake remote repo+branch
_push_new_commit() {
  local _repo=$1 _branch=$2 _file=$3 _msg=$4
  local _repo_dir="$DIR_BASE/git_repos/$_repo"

  (
    cd "$_repo_dir"
    git checkout "$_branch" 2>/dev/null
    echo "$_msg" >> "$_file"
    git add "$_file"
    git config user.name "Test User"
    git config user.email "test@example.com"
    git commit -m "$_msg"
    git push origin "$_branch"
    git checkout main 2>/dev/null
  )
}

# Push a new tag to a fake remote
_push_new_tag() {
  local _repo=$1 _tag=$2
  local _repo_dir="$DIR_BASE/git_repos/$_repo"

  (
    cd "$_repo_dir"
    git tag "$_tag"
    git push origin "$_tag"
  )
}

# Delete a branch on the fake remote (and local tracking ref)
_delete_remote_branch() {
  local _repo=$1 _branch=$2
  local _repo_dir="$DIR_BASE/git_repos/$_repo"

  (
    cd "$_repo_dir"
    git push origin --delete "$_branch"
  )
}

# Run check-push.sh --once with the fake docker binary
_run_check_push() {
  rm -rf "$CI_LOCK"
  PATH="$FAKE_DOCKER_DIR:$PATH" bash "$CHECK_PUSH_SCRIPT" --once
}

# ---------------------------------------------------------------------------
# 0. Snapshot state after the first run (baseline for idempotency checks)
# ---------------------------------------------------------------------------
echo "--- Recording baseline state after first run ---"

REV_WEBAPP_MAIN_BEFORE=$(_read_git_rev "webapp.main")
REV_API_DEV_BEFORE=$(_read_git_rev "api-service.dev")
REV_MOBILE_MAIN_BEFORE=$(_read_git_rev "mobile-app.main")

LATEST_WEBAPP_BEFORE=$(readlink "$DIR_BASE/copies/webapp.prod.latest" 2>/dev/null || true)

[[ -n "$REV_WEBAPP_MAIN_BEFORE" ]] || _fail "webapp.main has no .git-rev after first run"
[[ -n "$LATEST_WEBAPP_BEFORE" ]]   || _fail "webapp.prod.latest symlink missing after first run"

_ok "baseline recorded (webapp.main rev=${REV_WEBAPP_MAIN_BEFORE:0:8}..., latest=$LATEST_WEBAPP_BEFORE)"

# ---------------------------------------------------------------------------
# 1. Push new commits to remote branches
# ---------------------------------------------------------------------------
echo ""
echo "--- Scenario 1: push new commits to remote branches ---"

_push_new_commit "webapp" "main" "new-feature.txt" "Add new feature after first run"
_push_new_commit "api-service" "dev" "hotfix.txt" "Hotfix on dev branch"

echo "  commits pushed, running check-push.sh..."
_run_check_push

REV_WEBAPP_MAIN_AFTER=$(_read_git_rev "webapp.main")
REV_API_DEV_AFTER=$(_read_git_rev "api-service.dev")

[[ "$REV_WEBAPP_MAIN_AFTER" != "$REV_WEBAPP_MAIN_BEFORE" ]] || \
  _fail "webapp.main .git-rev did not change after new commit"
_ok "webapp.main .git-rev updated (${REV_WEBAPP_MAIN_BEFORE:0:8} -> ${REV_WEBAPP_MAIN_AFTER:0:8})"

# api-service.dev has .debugging from create-test-scenarios, so it should NOT update
[[ "$REV_API_DEV_AFTER" == "$REV_API_DEV_BEFORE" ]] || \
  _fail "api-service.dev .git-rev changed despite .debugging flag"
_ok "api-service.dev .git-rev unchanged (respected .debugging)"

# Verify the new file actually landed in the copy dir
[[ -f "$DIR_BASE/copies/webapp.main/new-feature.txt" ]] || \
  _fail "webapp.main copy missing new-feature.txt after update"
_ok "webapp.main copy contains new file (new-feature.txt)"

# ---------------------------------------------------------------------------
# 2. Push a new tag to remote — new release copy + symlink update
# ---------------------------------------------------------------------------
echo ""
echo "--- Scenario 2: push new tag to remote ---"

_push_new_tag "webapp" "v2026Q2.0.0"

echo "  tag pushed, running check-push.sh..."
_run_check_push

[[ -d "$DIR_BASE/copies/webapp.prod.v2026Q2.0.0" ]] || \
  _fail "webapp.prod.v2026Q2.0.0 copy dir not created for new tag"
_ok "webapp.prod.v2026Q2.0.0 release copy created"

LATEST_WEBAPP_AFTER=$(readlink "$DIR_BASE/copies/webapp.prod.latest" 2>/dev/null || true)
[[ "$LATEST_WEBAPP_AFTER" == "webapp.prod.v2026Q2.0.0" ]] || \
  _fail "webapp.prod.latest points to '$LATEST_WEBAPP_AFTER', expected 'webapp.prod.v2026Q2.0.0'"
_ok "webapp.prod.latest symlink updated to v2026Q2.0.0"

# A freshly-created tag copy must carry a .git-rev matching the tag's commit
REV_NEW_TAG=$(_read_git_rev "webapp.prod.v2026Q2.0.0")
[[ -n "$REV_NEW_TAG" ]] || \
  _fail "webapp.prod.v2026Q2.0.0 has no .git-rev after tag copy creation"
_ok "webapp.prod.v2026Q2.0.0 .git-rev written on creation (${REV_NEW_TAG:0:8})"

EXPECTED_TAG_REV=$( cd "$DIR_BASE/git_repos/webapp" && git rev-parse "v2026Q2.0.0" )
[[ "$REV_NEW_TAG" == "$EXPECTED_TAG_REV" ]] || \
  _fail "webapp.prod.v2026Q2.0.0 .git-rev='$REV_NEW_TAG' != tag rev '$EXPECTED_TAG_REV'"
_ok "webapp.prod.v2026Q2.0.0 .git-rev matches tag commit"

# TOPN=4 — the oldest release should have been pruned (v2.1 was already gone, next is v10.0)
# After adding v2026Q2.0.0, the top 4 are: v2026Q2.0.0, v2026Q1.0.0, v2025Q12.1.0, v2025Q4.2.0
# So v10.0 should now be marked to-be-removed (no longer in top 4)
if [[ -d "$DIR_BASE/copies/webapp.prod.v10.0.to-be-removed" ]] || \
   [[ ! -d "$DIR_BASE/copies/webapp.prod.v10.0" ]]; then
  _ok "webapp.prod.v10.0 pruned after TOPN shift (to-be-removed or gone)"
else
  _fail "webapp.prod.v10.0 still present after TOPN shifted to 4 newer releases"
fi

# ---------------------------------------------------------------------------
# 2b. Pre-existing tag copy missing .git-rev — must be backfilled (self-heal)
# ---------------------------------------------------------------------------
# Tag copies are immutable: checkout_and_copy_tag returns early when the dir
# already exists. Copies created before .git-rev support (or otherwise missing
# the file) must get it backfilled on the next run, not skipped forever.
echo ""
echo "--- Scenario 2b: backfill .git-rev for pre-existing tag copy ---"

TAG_COPY="$DIR_BASE/copies/webapp.prod.v2026Q2.0.0"
rm -f "$TAG_COPY/.git-rev"
[[ ! -f "$TAG_COPY/.git-rev" ]] || _fail "could not simulate missing .git-rev"
_ok "removed .git-rev to simulate a pre-fix tag copy"

echo "  running check-push.sh (no remote changes)..."
_run_check_push

REV_BACKFILLED=$(_read_git_rev "webapp.prod.v2026Q2.0.0")
[[ -n "$REV_BACKFILLED" ]] || \
  _fail "webapp.prod.v2026Q2.0.0 .git-rev not backfilled on re-run (early-return skipped it)"
[[ "$REV_BACKFILLED" == "$EXPECTED_TAG_REV" ]] || \
  _fail "backfilled .git-rev='$REV_BACKFILLED' != tag rev '$EXPECTED_TAG_REV'"
_ok "webapp.prod.v2026Q2.0.0 .git-rev backfilled (${REV_BACKFILLED:0:8})"

# ---------------------------------------------------------------------------
# 3. Delete a branch from remote — copy should become to-be-removed
# ---------------------------------------------------------------------------
echo ""
echo "--- Scenario 3: delete branch from remote ---"

# First make sure the test branch copy exists (it was .skipping, but let's verify the dir is there)
# We'll use the 'dev' branch on mobile-app since it has an active copy without special flags
REV_MOBILE_DEV_BEFORE=$(_read_git_rev "mobile-app.dev")

_delete_remote_branch "mobile-app" "dev"

echo "  branch deleted, running check-push.sh..."
_run_check_push

# After fetch --prune, origin/dev is gone. The cleanup loop should mark it .to-be-removed
# because it no longer gets a .living heartbeat.
# Note: this takes TWO runs — first run removes .living, second run sees no .living → to-be-removed.
# So we need one more run.
echo "  running check-push.sh again (second pass for cleanup)..."
_run_check_push

if [[ -d "$DIR_BASE/copies/mobile-app.dev.to-be-removed" ]] || \
   [[ ! -d "$DIR_BASE/copies/mobile-app.dev" ]]; then
  _ok "mobile-app.dev marked to-be-removed after branch deletion"
else
  _fail "mobile-app.dev still exists as normal dir after branch deleted from remote"
fi

# ---------------------------------------------------------------------------
# 4. Idempotency — re-run with no changes, revs should be stable
# ---------------------------------------------------------------------------
echo ""
echo "--- Scenario 4: idempotency (no-op re-run) ---"

REV_WEBAPP_MAIN_PRECHECK=$(_read_git_rev "webapp.main")
REV_MOBILE_MAIN_PRECHECK=$(_read_git_rev "mobile-app.main")
LATEST_WEBAPP_PRECHECK=$(readlink "$DIR_BASE/copies/webapp.prod.latest" 2>/dev/null || true)

echo "  running check-push.sh with no remote changes..."
_run_check_push

REV_WEBAPP_MAIN_POSTCHECK=$(_read_git_rev "webapp.main")
REV_MOBILE_MAIN_POSTCHECK=$(_read_git_rev "mobile-app.main")
LATEST_WEBAPP_POSTCHECK=$(readlink "$DIR_BASE/copies/webapp.prod.latest" 2>/dev/null || true)

[[ "$REV_WEBAPP_MAIN_POSTCHECK" == "$REV_WEBAPP_MAIN_PRECHECK" ]] || \
  _fail "webapp.main .git-rev changed on no-op re-run"
_ok "webapp.main .git-rev stable on no-op re-run"

[[ "$REV_MOBILE_MAIN_POSTCHECK" == "$REV_MOBILE_MAIN_PRECHECK" ]] || \
  _fail "mobile-app.main .git-rev changed on no-op re-run"
_ok "mobile-app.main .git-rev stable on no-op re-run"

[[ "$LATEST_WEBAPP_POSTCHECK" == "$LATEST_WEBAPP_PRECHECK" ]] || \
  _fail "webapp.prod.latest symlink changed on no-op re-run"
_ok "webapp.prod.latest symlink stable on no-op re-run"

# ---------------------------------------------------------------------------
echo ""
echo "=== All multi-run integration tests passed ==="
