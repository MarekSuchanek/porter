#!/usr/bin/env bash
# Sets the version of the CLI and the desktop app (both read Cargo.toml's
# [workspace.package] version) and of package.json, then prints the release steps.
#
#   scripts/bump-version.sh 0.2.0
set -euo pipefail

version="${1:-}"
if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
  echo "usage: $0 X.Y.Z[-pre]" >&2
  exit 1
fi
cd "$(dirname "$0")/.."

# Only the version line inside [workspace.package].
sed -i.bak "/^\[workspace.package\]/,/^\[/ s/^version = \".*\"/version = \"$version\"/" Cargo.toml
rm Cargo.toml.bak
npm pkg set version="$version"
cargo update --workspace --quiet # refresh Cargo.lock

echo "Version set to $version. Next:"
echo "  1. Add a '## [$version] - $(date +%F)' section to CHANGELOG.md"
echo "  2. git commit -am 'Release $version'"
echo "  3. git tag v$version && git push origin main v$version"
echo "  4. Review the draft release on GitHub, then publish it"
