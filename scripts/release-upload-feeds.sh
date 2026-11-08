#!/usr/bin/env bash
# Upload the archives to R2, then deploy the worker that serves the feeds.
#
# Usage: release-upload-feeds.sh VERSION
#
# Preconditions: build/release holds both signed archives. The feeds and their
# _headers file are in build/update-assets. cloudflare/ has its dependencies.
set -euo pipefail

version="${1:-}"
version="${version#v}"
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "usage: release-upload-feeds.sh MAJOR.MINOR.PATCH" >&2
  exit 1
fi
: "${CLOUDFLARE_API_TOKEN:?CLOUDFLARE_API_TOKEN is required}"

cd "$(dirname "$0")/.."
release_dir=$PWD/build/release
cd cloudflare

for arch in arm64 x86_64; do
  archive="LitematicaQL-$version-$arch.zip"
  npx --no-install cf r2 objects put "litematicaql/$archive" \
    --bucket-name apps-download --file "$release_dir/$archive" \
    --content-type application/zip
done

npx --no-install cf deploy
