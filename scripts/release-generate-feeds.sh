#!/usr/bin/env bash
# Generate the signed appcasts for the built archives.
#
# Usage: release-generate-feeds.sh VERSION
#
# Preconditions: build/release holds both signed archives with their checksums.
# SPARKLE_PRIVATE_KEY is the EdDSA key. GH_TOKEN can read the Sparkle release.
set -euo pipefail

version="${1:-}"
version="${version#v}"
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "usage: release-generate-feeds.sh MAJOR.MINOR.PATCH" >&2
  exit 1
fi
: "${SPARKLE_PRIVATE_KEY:?SPARKLE_PRIVATE_KEY is required}"

cd "$(dirname "$0")/.."
work_dir=$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/litematicaql-feeds.XXXXXX")
trap 'rm -rf "$work_dir"' EXIT

(cd build/release && shasum -a 256 -c \
  "LitematicaQL-$version-arm64.zip.sha256" \
  "LitematicaQL-$version-x86_64.zip.sha256")

# The feed tools come from the pinned Sparkle release. The checksum is the one
# published with that release.
gh release download 2.9.0 --repo sparkle-project/Sparkle \
  --pattern Sparkle-2.9.0.tar.xz --dir "$work_dir/sparkle"
echo "01e0f0ebf6614061ea816d414de50f937d64ffa6822ad572243031ca3676fe19  $work_dir/sparkle/Sparkle-2.9.0.tar.xz" |
  shasum -a 256 -c
tar -xJf "$work_dir/sparkle/Sparkle-2.9.0.tar.xz" -C "$work_dir/sparkle"

rm -rf build/update-assets
for arch in arm64 x86_64; do
  archive="LitematicaQL-$version-$arch.zip"
  feed_dir="$work_dir/appcast/$arch"
  mkdir -p "$feed_dir" "build/update-assets/litematicaql/$arch"
  cp "build/release/$archive" "$feed_dir/$archive"
  printf '%s' "$SPARKLE_PRIVATE_KEY" | "$work_dir/sparkle/bin/generate_appcast" \
    --ed-key-file - --maximum-deltas 0 \
    --download-url-prefix "https://download.arcadia.moe/litematicaql/" \
    -o "$feed_dir/appcast.xml" "$feed_dir"

  appcast="$feed_dir/appcast.xml"
  enclosures=$(xmllint --xpath 'count(//*[local-name()="item"]/*[local-name()="enclosure"])' "$appcast")
  signature=$(xmllint --xpath 'string(//*[local-name()="enclosure"]/@*[local-name()="edSignature"])' "$appcast")
  if [[ "$enclosures" != 1 || -z "$signature" ]]; then
    echo "Generated appcast does not contain a signed update." >&2
    exit 1
  fi
  printf '%s' "$SPARKLE_PRIVATE_KEY" | "$work_dir/sparkle/bin/sign_update" \
    --verify --ed-key-file - "$feed_dir/$archive" "$signature"

  cp "$appcast" "build/update-assets/litematicaql/$arch/appcast.xml"
done

printf '/litematicaql/*\n  Cache-Control: public, max-age=300, must-revalidate\n' \
  > build/update-assets/_headers
