#!/usr/bin/env bash

set -euo pipefail
cd "$(dirname "$0")/.."

if [[ $# -ne 1 ]] || ! [[ $1 =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "usage: $0 MAJOR.MINOR.PATCH" >&2
    exit 1
fi
version=$1

sed -i '' "s/^MARKETING_VERSION: .*/MARKETING_VERSION: ${version}/" project.yml
sed -i '' "s/^  \"version\": \".*\"/  \"version\": \"${version}\"/" Renderer/package.json

grep -q "MARKETING_VERSION: ${version}" project.yml
grep -q "\"version\": \"${version}\"" Renderer/package.json

if ! command -v xcodegen >/dev/null 2>&1; then
    echo "xcodegen is required to regenerate LitematicaQL.xcodeproj" >&2
    exit 1
fi
xcodegen generate >/dev/null
grep -q "MARKETING_VERSION = ${version};" LitematicaQL.xcodeproj/project.pbxproj

echo "Version bumped to ${version}. Commit with: chore: bump to v${version}"
