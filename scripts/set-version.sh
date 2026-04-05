#!/usr/bin/env bash
# Set project version everywhere. Usage: ./scripts/set-version.sh 1.2.3
# Updates: Cargo.toml, deployment/docker-compose/compose.yml, and the embedded check-push.sh script
set -e
if [[ -z "${1:-}" ]]; then
  echo "Usage: $0 <version>" >&2
  echo "Example: $0 1.2.3" >&2
  exit 1
fi
v=$1

if [[ ! "$v" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "Version format is invalid: $v, must be in the format of x.y.z (e.g. 1.2.3)" >&2
  exit 1
fi

# update version in Cargo.toml and Cargo.lock
root=$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)
sed -i.bak -E "s/^version = \".+\"/version = \"$v\"/" "$root/Cargo.toml" && rm -f "$root/Cargo.toml.bak"
# trigger version update in Cargo.lock
cargo update --package git-supervisor --precise "$v"

# Update version in deployment/docker-compose/compose.yml (image tag)
compose_yml="$root/deployment/docker-compose/compose.yml"
if [[ -f "$compose_yml" ]]; then
  sed -i.bak -E "s|(rushiai/git-supervisor:)[^\"']+|\1v$v|g" "$compose_yml" && rm -f "$compose_yml.bak"
else
  echo "Warning: $compose_yml not found, skipping docker-compose version update" >&2
fi

# update version string in core/check-push.sh
check_push_sh="$root/core/check-push.sh"
if [[ -f "$check_push_sh" ]]; then
  sed -i.bak -E "s/^VERSION=\"[^\"]+\"/VERSION=\"$v\"/" "$check_push_sh" && rm -f "$check_push_sh.bak"
else
  echo "Warning: $check_push_sh not found, skipping core/check-push.sh version update" >&2
fi

echo "Version set to $v in Cargo.toml/lock, core/check-push.sh and reference (docker)compose.yml"
