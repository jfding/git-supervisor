#!/bin/bash

# Test script for check-push.sh
# This script runs the check-push.sh script with test configuration

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEV_DIR="$(cd "$(dirname "$SCRIPT_DIR")/work.test" && pwd)"
CHECK_PUSH_SCRIPT="$DEV_DIR/../../src/check-push.sh"

# Test configuration (script uses DIR_BASE and derives DIR_REPOS, DIR_COPIES; CI_LOCK is a directory)
export DIR_BASE="$DEV_DIR"
export CI_LOCK="$DEV_DIR/.ci-lock.d"
export VERB=2  # Verbose output
export TIMEOUT=30  # Shorter timeout for testing
export SLEEP_TIME=""  # Run once and exit

echo "=== Testing check-push.sh ==="
echo "DIR_BASE: $DIR_BASE"
echo "Git repos directory: $DIR_BASE/git_repos"
echo "Copies directory: $DIR_BASE/copies"
echo "CI lock directory: $CI_LOCK"
echo ""

# Check if check-push.sh exists
if [[ ! -f "$CHECK_PUSH_SCRIPT" ]]; then
    echo "Error: check-push.sh not found at $CHECK_PUSH_SCRIPT"
    exit 1
fi

# Check if test repositories exist
if [[ ! -d "$DIR_BASE/git_repos" ]] || [[ -z "$(ls -A "$DIR_BASE/git_repos" 2>/dev/null)" ]]; then
    echo "Error: No test repositories found in $DIR_BASE/git_repos"
    echo "Please run setup-test-repos.sh first"
    exit 1
fi

# Clean up any existing lock directory (check-push.sh uses mkdir/rmdir for CI_LOCK)
rm -rf "$CI_LOCK"

# Make check-push.sh executable
chmod +x "$CHECK_PUSH_SCRIPT"

echo "Running check-push.sh..."
echo ""

# Run the script (BR_WHITELIST from env when run via launch-testing.sh / test-config.sh)
bash "$CHECK_PUSH_SCRIPT" --once

echo ""
echo "=== Verifying release tag ordering ==="
_expected_latest="v2026Q1.0.0"
for _repo in webapp api-service mobile-app; do
  _latest_link="$DIR_BASE/copies/${_repo}.prod.latest"
  _latest_target=$(readlink "$_latest_link" 2>/dev/null || true)

  if [[ "$_latest_target" != "${_repo}.prod.${_expected_latest}" ]]; then
    echo "Error: ${_repo}.prod.latest points to '${_latest_target}', expected '${_repo}.prod.${_expected_latest}'"
    exit 1
  fi

  for _tag in v2025Q12.1.0 v2025Q4.2.0 v10.0; do
    if [[ ! -d "$DIR_BASE/copies/${_repo}.prod.${_tag}" ]]; then
      echo "Error: missing expected release copy for ${_repo}.prod.${_tag}"
      exit 1
    fi
  done

  if [[ -d "$DIR_BASE/copies/${_repo}.prod.v2.1" ]]; then
    echo "Error: ${_repo}.prod.v2.1 should not exist because RELEASE_TAG_TOPN keeps only the top 4 releases"
    exit 1
  fi

  echo "  OK: $_repo release tags sorted correctly, latest=${_expected_latest}"
done

echo ""
echo "=== Verifying BR_WHITELIST / .skipping behavior ==="
# Whitelisted branches should have copy dir with content and no .skipping when just inited
_found_whitelisted_ok=0
for _d in "$DIR_BASE/copies"/*.*; do
  [[ -d "$_d" ]] || continue
  _base=$(basename "$_d")
  # skip special dirs
  [[ $_base == *".latest" ]] && continue
  [[ $_base == *".to-be-removed" ]] && continue
  [[ $_base == *".staging."* ]] && continue
  if [[ -f "$_d/.git-rev" ]] && [[ ! -f "$_d/.skipping" ]]; then
    _found_whitelisted_ok=1
    echo "  OK: $_base has .git-rev and no .skipping (whitelisted init or updated)"
    break
  fi
done
if [[ $_found_whitelisted_ok -eq 0 ]]; then
  echo "  WARN: no copy dir found with .git-rev and without .skipping (check repos/whitelist)"
fi
# Opt-in branch copy (e.g. from create-test-scenarios) should keep .skipping
if [[ -d "$DIR_BASE/copies/webapp.test" ]] && [[ -f "$DIR_BASE/copies/webapp.test/.skipping" ]]; then
  echo "  OK: webapp.test keeps .skipping (opt-in scenario preserved)"
fi

echo ""
echo "=== Test completed ==="
echo "Check the copies directory for results:"
ls -la "$DIR_BASE/copies"
