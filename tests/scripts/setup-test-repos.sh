#!/bin/bash

# Setup script to create mock Git repositories for testing check-push.sh
# This script creates test repositories with various branches and tags

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEV_DIR="$(dirname "$SCRIPT_DIR")/work.test"
GIT_REPOS_DIR="$DEV_DIR/git_repos"
COPIES_DIR="$DEV_DIR/copies"
FAKE_REMOTES_DIR="$DEV_DIR/fake-remotes"

echo "Setting up test repositories in $GIT_REPOS_DIR"
echo "Creating fake remotes in $FAKE_REMOTES_DIR"

# Reset previous test state so release-tag assertions are deterministic.
rm -rf "$GIT_REPOS_DIR" "$COPIES_DIR" "$FAKE_REMOTES_DIR"

# Create directories
mkdir -p "$GIT_REPOS_DIR"
mkdir -p "$COPIES_DIR"
mkdir -p "$FAKE_REMOTES_DIR"

# Function to create a fake remote repository
create_fake_remote() {
    local repo_name=$1
    local remote_dir="$FAKE_REMOTES_DIR/$repo_name.git"
    
    echo "Creating fake remote: $repo_name"
    
    # Remove if exists
    rm -rf "$remote_dir"
    mkdir -p "$remote_dir"
    cd "$remote_dir"
    
    # Initialize bare repository (like GitHub)
    git init --bare
    git config user.name "Test Remote"
    git config user.email "remote@test.com"
    
    echo "Created fake remote: $repo_name"
}

# Function to create a test repository
create_test_repo() {
    local repo_name=$1
    local repo_dir="$GIT_REPOS_DIR/$repo_name"
    local remote_dir="$FAKE_REMOTES_DIR/$repo_name.git"
    
    echo "Creating test repository: $repo_name"
    
    # Remove if exists
    rm -rf "$repo_dir"
    mkdir -p "$repo_dir"
    cd "$repo_dir"
    
    # Initialize git repo on main regardless of global git defaults.
    # Fallback keeps compatibility with older git versions lacking -b.
    if ! git init -b main >/dev/null 2>&1; then
        git init
        git checkout -b main
    fi
    git config user.name "Test User"
    git config user.email "test@example.com"
    
    # Create initial files
    echo "# $repo_name" > README.md
    echo "This is a test repository for $repo_name" >> README.md
    echo "Version: 1.0.0" > version.txt
    echo "#!/bin/bash" > deploy.sh
    echo "echo 'Deploying $repo_name...'" >> deploy.sh
    chmod +x deploy.sh
    
    # Initial commit
    git add .
    git commit -m "Initial commit for $repo_name"
    
    # Create branches
    git checkout -b dev
    echo "Development branch" > dev-info.txt
    git add dev-info.txt
    git commit -m "Add dev branch"
    
    git checkout -b test
    echo "Test branch" > test-info.txt
    git add test-info.txt
    git commit -m "Add test branch"
    
    git checkout -b feature/new-feature
    echo "New feature" > feature.txt
    git add feature.txt
    git commit -m "Add new feature"
    
    # Go back to main
    git checkout main
    
    # Create tags
    git tag v1.0.0
    git tag v1.1.0
    git tag v2.0.0
    git tag v2.0.0.1
    git tag v2.1
    git tag v10.0
    git tag v1.2.Q3
    git tag v1.2.Q10
    git tag v2025Q4.2.0
    git tag v2025Q12.1.0
    git tag v2026Q1.0.0
    
    # Add the fake remote
    git remote add origin "$remote_dir"
    
    # Push all branches and tags to the fake remote
    git push origin main
    git push origin dev
    git push origin test
    git push origin feature/new-feature
    git push origin --tags
    
    # Fetch from remote to create proper remote tracking branches
    git fetch origin
    
    # Set up remote tracking branches properly
    git branch --set-upstream-to=origin/main main
    git branch --set-upstream-to=origin/dev dev
    git branch --set-upstream-to=origin/test test
    git branch --set-upstream-to=origin/feature/new-feature feature/new-feature
    
    echo "Created repository: $repo_name with local remote"
}

# Create fake remotes first
create_fake_remote "webapp"
create_fake_remote "api-service"
create_fake_remote "mobile-app"

# Create test repositories
create_test_repo "webapp"
create_test_repo "api-service"
create_test_repo "mobile-app"

echo ""
echo "Test repositories created successfully!"
echo "Git repos directory: $GIT_REPOS_DIR"
echo "Fake remotes directory: $FAKE_REMOTES_DIR"
echo "Copies directory: $COPIES_DIR"
echo ""
echo "Each repository now has a local bare repository as its 'origin' remote."
