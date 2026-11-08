#!/usr/bin/env bash
# Publish the signed archives and their checksums as a GitHub release.
#
# Usage: release-publish-github.sh VERSION
#
# Preconditions: build/release holds both signed archives with their checksums.
# APP_NAME is the application name. GH_TOKEN can write repository releases.
set -euo pipefail

version="${1:-}"
if [[ ! "$version" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "usage: release-publish-github.sh vMAJOR.MINOR.PATCH" >&2
  exit 1
fi
: "${APP_NAME:?APP_NAME is required}"
: "${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is required}"

cd "$(dirname "$0")/.."
release_dir=$PWD/build/release
package_version="${version#v}"

(cd "$release_dir" && sha256sum -c \
  "$APP_NAME-$package_version-arm64.zip.sha256" \
  "$APP_NAME-$package_version-x86_64.zip.sha256")

gh release create "$version" \
  "$release_dir/$APP_NAME-$package_version-arm64.zip" \
  "$release_dir/$APP_NAME-$package_version-arm64.zip.sha256" \
  "$release_dir/$APP_NAME-$package_version-x86_64.zip" \
  "$release_dir/$APP_NAME-$package_version-x86_64.zip.sha256" \
  --repo "$GITHUB_REPOSITORY" \
  --verify-tag \
  --title "$APP_NAME $version" \
  --generate-notes
