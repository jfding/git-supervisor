#!/usr/bin/env bash
# Set project version everywhere. Usage: ./scripts/set-version.sh 1.2.3
# Updates: VERSION, supervisor/Cargo.toml, deployment/docker-compose/compose.yml
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

root=$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)
echo "$v" > "$root/VERSION"
sed -i.bak -E "s/^version = \".+\"/version = \"$v\"/" "$root/supervisor/Cargo.toml" && rm -f "$root/supervisor/Cargo.toml.bak"

# Update version in deployment/docker-compose/compose.yml (image tag)
compose_yml="$root/deployment/docker-compose/compose.yml"
if [[ -f "$compose_yml" ]]; then
  sed -i.bak -E "s|(rushiai/auto-reloader:)[^\"']+|\1v$v|g" "$compose_yml" && rm -f "$compose_yml.bak"
else
  echo "Warning: $compose_yml not found, skipping docker-compose version update" >&2
fi

# trigger version update in Cargo.lock
cd "$root/supervisor" && cargo update --package git-supervisor --precise "$v"

echo "Version set to $v in VERSION, supervisor/Cargo.toml, and reference (docker)compose.yml"
