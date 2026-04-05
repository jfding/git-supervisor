#!/bin/bash

# Launch script for testing check-push.sh
# This script sets up and runs the complete test environment

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
DEV_TESTING_DIR="$PROJECT_ROOT/tests/work.test"
TESTING_SCRIPTS_DIR="$PROJECT_ROOT/tests/scripts"

echo "=== Tripitaka Auto-Reloader Test Environment ==="
echo "Project root: $PROJECT_ROOT"
echo "Dev testing directory: $DEV_TESTING_DIR"
echo "Testing scripts directory: $TESTING_SCRIPTS_DIR"
echo ""

# Load test configuration
echo "Loading test configuration..."
source "$TESTING_SCRIPTS_DIR/test-config.sh"

echo ""
echo "Available test commands:"
echo "1. Setup test repositories: ./scripts/setup-test-repos.sh"
echo "2. Create test scenarios: ./scripts/create-test-scenarios.sh"
echo "3. Run check-push test: ./scripts/test-check-push.sh"
echo "4. Cleanup test environment: ./scripts/cleanup-test.sh"
echo ""

# Check if test repositories exist
if [[ ! -d "$DIR_BASE/git_repos" ]] || [[ -z "$(ls -A "$DIR_BASE/git_repos" 2>/dev/null)" ]]; then
    echo "Test repositories not found. Setting up..."
    "$TESTING_SCRIPTS_DIR/setup-test-repos.sh"
    echo ""
fi

# Check if test scenarios exist
if [[ ! -d "$DIR_BASE/copies" ]] || [[ -z "$(ls -A "$DIR_BASE/copies" 2>/dev/null)" ]]; then
    echo "Test scenarios not found. Creating..."
    "$TESTING_SCRIPTS_DIR/create-test-scenarios.sh"
    echo ""
fi

echo "Running check-push.sh test..."
echo ""

# Run the test
"$TESTING_SCRIPTS_DIR/test-check-push.sh"

echo ""
echo "=== Test completed ==="
echo "Check the tests/work.test/copies directory for results."
echo "Use './scripts/cleanup-test.sh' to clean up when done."
