#!/usr/bin/env bash
# Refuse to replace a published package with different bytes.
#
# Usage: release-check-packages.sh VERSION
#
# Preconditions: build/release holds both signed archives.
# CLOUDFLARE_API_TOKEN can read the apps-download bucket. cloudflare/ has its
# dependencies installed.
set -euo pipefail

version="${1:-}"
version="${version#v}"
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "usage: release-check-packages.sh MAJOR.MINOR.PATCH" >&2
  exit 1
fi
: "${CLOUDFLARE_API_TOKEN:?CLOUDFLARE_API_TOKEN is required}"

cd "$(dirname "$0")/.."
release_dir=$PWD/build/release
work_dir=$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/litematicaql-packages.XXXXXX")
trap 'rm -rf "$work_dir"' EXIT
cd cloudflare

for arch in arm64 x86_64; do
  archive="LitematicaQL-$version-$arch.zip"
  if npx --no-install cf r2 objects get "litematicaql/$archive" \
    --bucket-name apps-download -q > "$work_dir/$arch" 2> "$work_dir/$arch.err"; then
    if ! cmp -s "$release_dir/$archive" "$work_dir/$arch"; then
      echo "Published package $archive differs from the built archive; refusing to overwrite it." >&2
      exit 1
    fi
  elif ! grep -q '10007' "$work_dir/$arch.err"; then
    echo "Cannot read the published package $archive:" >&2
    cat "$work_dir/$arch.err" >&2
    exit 1
  fi
done
