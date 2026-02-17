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

# Run the script
bash "$CHECK_PUSH_SCRIPT" once

echo ""
echo "=== Test completed ==="
echo "Check the copies directory for results:"
ls -la "$DIR_BASE/copies"
