#!/usr/bin/env bash
# Refuse to replace a published package with different bytes.
#
# Usage: release-check-packages.sh VERSION [PACKAGE_BASE_URL]
set -euo pipefail

version="${1:-}"
base_url="${2:-https://download.arcadia.moe/litematicaql}"
if [[ ! "$version" =~ ^v?[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "usage: release-check-packages.sh vMAJOR.MINOR.PATCH [PACKAGE_BASE_URL]" >&2
  exit 1
fi
version="${version#v}"

source "$(dirname "$0")/release-http.sh"
cd "$(dirname "$0")/.."

work_dir=$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/litematicaql-packages.XXXXXX")
trap 'rm -rf "$work_dir"' EXIT

for arch in arm64 x86_64; do
  archive="LitematicaQL-$version-$arch.zip"
  url="$base_url/$archive?run=${GITHUB_RUN_ID:-local}-${GITHUB_RUN_ATTEMPT:-0}"
  status=$(fetch_public "$url" "$work_dir/$arch")
  if [[ "$status" == 404 ]]; then
    continue
  fi
  reject_unavailable "$url" "$status"
  if ! cmp -s "build/release/$archive" "$work_dir/$arch"; then
    echo "Package $url differs from the built archive; refusing to overwrite it." >&2
    exit 1
  fi
done
