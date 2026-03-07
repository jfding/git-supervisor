#!/usr/bin/env bash
# Set project version everywhere. Usage: ./scripts/set-version.sh 1.2.3
# Updates: VERSION, supervisor/Cargo.toml, gh-webhook/pyproject.toml
set -e
if [[ -z "${1:-}" ]]; then
  echo "Usage: $0 <version>" >&2
  echo "Example: $0 1.2.3" >&2
  exit 1
fi
v=$1
root=$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)
echo "$v" > "$root/VERSION"
sed -i.bak -E "s/^version = \".+\"/version = \"$v\"/" "$root/supervisor/Cargo.toml" && rm -f "$root/supervisor/Cargo.toml.bak"
sed -i.bak -E "s/^version = \".+\"/version = \"$v\"/" "$root/gh-webhook/pyproject.toml" && rm -f "$root/gh-webhook/pyproject.toml.bak"
echo "Version set to $v in VERSION, supervisor/Cargo.toml, gh-webhook/pyproject.toml"
