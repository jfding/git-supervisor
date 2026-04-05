#!/bin/bash

# Script to create various test scenarios for check-push.sh
# This creates different configurations and test cases

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEV_DIR="$(dirname "$SCRIPT_DIR")/work.test"
COPIES_DIR="$DEV_DIR/copies"

echo "Creating test scenarios..."

# Create Docker restart files for testing
create_docker_script() {
    local repo_name=$1
    local branch=$2
    local docker_name="${repo_name}-${branch}"
    local docker_path="$COPIES_DIR/${repo_name}.${branch}.docker"
    
    echo "$docker_name" > "$docker_path"
    echo "Created docker file: $docker_path (container: $docker_name)"
}

create_docker_hook_jobs() {
    local docker_path=$1

    cat > "${docker_path}.pre" <<'EOF'
#!/usr/bin/env bash
set -e
printf '%s:%s\n' "${DOCKER_HOOK_STAGE:-}" "${DOCKER_NAME:-}" >> "${PWD}/.docker-hook-pre.txt"
EOF

    cat > "${docker_path}.post" <<'EOF'
#!/usr/bin/env bash
set -e
printf '%s:%s\n' "${DOCKER_HOOK_STAGE:-}" "${DOCKER_NAME:-}" >> "${PWD}/.docker-hook-post.txt"
EOF

    echo "Created docker hook jobs: ${docker_path}.pre and ${docker_path}.post"
}

# Create various test scenarios
echo "Creating Docker restart files..."
create_docker_script "webapp" "main"
create_docker_script "api-service" "main"
echo "webapp-prod" > "$COPIES_DIR/webapp.prod.docker"
echo "Created docker file: $COPIES_DIR/webapp.prod.docker (container: webapp-prod)"
create_docker_hook_jobs "$COPIES_DIR/webapp.main.docker"
create_docker_hook_jobs "$COPIES_DIR/webapp.prod.docker"

# Create special test files
echo "Creating special test files..."

# Create a .skipping file for a branch (webapp.test is not in default BR_WHITELIST; opt-in scenario)
mkdir -p "$COPIES_DIR/webapp.test"
touch "$COPIES_DIR/webapp.test/.skipping"
echo "Created .skipping file for webapp.test"

# Create a .debugging file for a branch
mkdir -p "$COPIES_DIR/api-service.dev"
touch "$COPIES_DIR/api-service.dev/.debugging"
echo "Created .debugging file for api-service.dev"

# Create a .no-cleanup file for a branch
mkdir -p "$COPIES_DIR/mobile-app.main"
touch "$COPIES_DIR/mobile-app.main/.no-cleanup"
echo "Created .no-cleanup file for mobile-app.main"

# Create a .trigger file for testing debug mode
mkdir -p "$COPIES_DIR/webapp.dev"
touch "$COPIES_DIR/webapp.dev/.trigger"
echo "Created .trigger file for webapp.dev"
mkdir -p "$COPIES_DIR/webapp.main"
touch "$COPIES_DIR/webapp.main/.trigger"
echo "seed" > "$COPIES_DIR/webapp.main/.seed"
echo "Created .trigger file for webapp.main (force docker restart/hook test)"

# Create dot prefixed dirs for tag
mkdir -p "$COPIES_DIR/.archives/api-service.prod.v1.0.0"
echo "Test content for dot-prefixed directory" > "$COPIES_DIR/.archives/api-service.prod.v1.0.0/test.txt"
echo "Created dot-prefixed directory: .archives/api-service.prod.v1.0.0"
mkdir -p "$COPIES_DIR/.archives/mobile-app.prod.v2.0.0"
echo "Test content for dot-prefixed directory" > "$COPIES_DIR/.archives/mobile-app.prod.v2.0.0/test.txt"
echo "Created dot-prefixed directory: .archives/mobile-app.prod.v2.0.0"

# Create invalid dirs for nonexisiting tags
mkdir -p "$COPIES_DIR/api-service.prod.v3.0.0"
echo "Test content for dot-prefixed directory" > "$COPIES_DIR/api-service.prod.v3.0.0/test.txt"
echo "Created dot-prefixed directory: api-service.prod.v3.0.0"
mkdir -p "$COPIES_DIR/mobile-app.prod.v3.0.0"
echo "Test content for dot-prefixed directory" > "$COPIES_DIR/mobile-app.prod.v3.0.0/test.txt"
echo "Created dot-prefixed directory: mobile-app.prod.v3.0.0"

echo ""
echo "Test scenarios created successfully!"
echo "Available test configurations:"
echo "- Docker restart configurations"
echo "- Docker pre/post hook jobs around restart"
echo "- .skipping file (webapp.test)"
echo "- .debugging file (api-service.dev)"
echo "- .no-cleanup file (mobile-app.main)"
echo "- .trigger files (webapp.dev, webapp.main)"
