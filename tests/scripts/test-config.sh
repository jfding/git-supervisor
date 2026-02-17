#!/bin/bash

# Test configuration for check-push.sh
# This file contains environment variables and settings for testing

# Resolve test work dir (tests/work.test) so paths work from any cwd
TEST_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export DIR_BASE="$(cd "$TEST_SCRIPT_DIR/../work.test" && pwd)"
# CI_LOCK is a directory in check-push.sh (mkdir/rmdir)
export CI_LOCK="$DIR_BASE/.ci-lock.d"

# Test settings
export VERB=2                    # Verbose output (0=silent, 1=normal, 2=verbose)
export TIMEOUT=30                # Shorter timeout for testing
export SLEEP_TIME=""             # Run once and exit (empty = no daemon mode)

# Branch whitelist for testing
export BR_WHITELIST="main master dev test alpha"

# Test mode flag
export TEST_MODE=1

echo "Test configuration loaded:"
echo "  DIR_BASE: $DIR_BASE"
echo "  DIR_REPOS: $DIR_BASE/git_repos (from script)"
echo "  DIR_COPIES: $DIR_BASE/copies (from script)"
echo "  CI_LOCK: $CI_LOCK (directory)"
echo "  VERB: $VERB"
echo "  TIMEOUT: $TIMEOUT"
echo "  SLEEP_TIME: $SLEEP_TIME"
echo "  BR_WHITELIST: $BR_WHITELIST"
echo "  TEST_MODE: $TEST_MODE"
